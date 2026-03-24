use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{Emitter, Manager};

use crate::query_parser;
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipResult {
    pub id: i64,
    pub content: String,
    pub content_type: String,
    pub source_app: String,
    pub created_at: i64,
    pub pinned: bool,
    pub source_app_icon: Option<String>,
    pub content_highlighted: Option<String>,
    pub ocr_text: Option<String>,
    pub image_thumbnail: Option<String>,
}

#[tauri::command]
pub async fn search_clips(
    app: tauri::AppHandle,
    query: String,
    filter: String,
) -> Result<Vec<ClipResult>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    if query.trim().is_empty() {
        return search_empty(&conn, &filter);
    }

    let parsed = query_parser::parse_query(&query);
    let sq = if parsed.semantic.is_empty() {
        &query
    } else {
        &parsed.semantic
    };

    // Build WHERE clauses
    let tc_join = match (parsed.temporal_after, parsed.temporal_before) {
        (Some(a), Some(b)) => format!(" AND c.created_at >= {} AND c.created_at <= {}", a, b),
        (Some(a), None) => format!(" AND c.created_at >= {}", a),
        (None, Some(b)) => format!(" AND c.created_at <= {}", b),
        (None, None) => String::new(),
    };
    let tc_plain = tc_join.replace("c.", "");

    let sa_join = parsed
        .source_app
        .as_ref()
        .map(|a| format!(" AND LOWER(c.source_app) LIKE '%{}%'", a.to_lowercase()))
        .unwrap_or_default();
    let sa_plain = sa_join.replace("c.", "");

    let ef = parsed.content_type.as_deref().unwrap_or(&filter);

    // Source-app-only query (e.g., "from Slack" with no other search terms)
    if sq.trim().is_empty() && parsed.source_app.is_some() {
        return search_by_source_app(&conn, parsed.source_app.as_deref().unwrap(), ef, &tc_plain);
    }

    // Run all search strategies in parallel
    let fts_results = do_search_fts(&conn, sq, ef, &tc_join, &sa_join);

    let vec_results = if let Some(ref model) = state.model {
        do_search_vec(model, &conn, sq, ef, &tc_join, &sa_join)
    } else {
        if !query.trim().is_empty() {
            eprintln!("[Search] Model not loaded — semantic search disabled");
        }
        vec![]
    };

    // LIKE fallback only if we have few results
    let like_results = if fts_results.len() + vec_results.len() < 10 {
        do_search_like(&conn, sq, ef, &tc_plain, &sa_plain)
    } else {
        vec![]
    };

    Ok(rrf(vec![fts_results, vec_results, like_results]))
}

#[tauri::command]
pub async fn get_total_clip_count(app: tauri::AppHandle) -> Result<i64, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row("SELECT COUNT(*) FROM clips", [], |row| row.get(0))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_pin(app: tauri::AppHandle, clip_id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let updated = conn
        .execute(
            "UPDATE clips SET pinned = CASE WHEN pinned = 1 THEN 0 ELSE 1 END WHERE id = ?",
            [clip_id],
        )
        .map_err(|e| e.to_string())?;
    drop(conn);

    if updated == 0 {
        return Err("Clip not found".into());
    }

    let _ = app.emit("clips-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn delete_clip(app: tauri::AppHandle, clip_id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM clip_embeddings WHERE rowid = ?", [clip_id])
        .map_err(|e| e.to_string())?;
    let deleted = conn
        .execute("DELETE FROM clips WHERE id = ?", [clip_id])
        .map_err(|e| e.to_string())?;
    drop(conn);

    if deleted == 0 {
        return Err("Clip not found".into());
    }

    let _ = app.emit("clips-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn get_image_thumbnail(
    app: tauri::AppHandle,
    clip_id: i64,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // Read the stored raw image data
    let result: Option<(Vec<u8>, Vec<u8>, i64, i64)> = conn
        .query_row(
            "SELECT COALESCE(image_thumbnail, X''), image_data, image_width, image_height FROM clips WHERE id = ? AND content_type = 'image'",
            [clip_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    match result {
        Some((thumbnail, _, _, _)) if !thumbnail.is_empty() => Ok(Some(b64(&thumbnail))),
        Some((_, raw_bytes, width, height)) if !raw_bytes.is_empty() => {
            let thumbnail = generate_thumbnail(&raw_bytes, width as u32, height as u32);
            Ok(thumbnail.map(|t| b64(&t)))
        }
        _ => Ok(None),
    }
}

#[tauri::command]
pub async fn get_image_preview(
    app: tauri::AppHandle,
    clip_id: i64,
    max_size: u32,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let result: Option<(Vec<u8>, i64, i64)> = conn
        .query_row(
            "SELECT image_data, image_width, image_height FROM clips WHERE id = ? AND content_type = 'image'",
            [clip_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    match result {
        Some((raw_bytes, width, height)) if !raw_bytes.is_empty() => {
            let preview = generate_thumbnail_sized(&raw_bytes, width as u32, height as u32, max_size);
            Ok(preview.map(|p| b64(&p)))
        }
        _ => Ok(None),
    }
}

fn generate_thumbnail(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    generate_thumbnail_sized(data, width, height, 64)
}

fn generate_thumbnail_sized(data: &[u8], width: u32, height: u32, max_size: u32) -> Option<Vec<u8>> {
    let scale = if width > max_size || height > max_size {
        max_size as f32 / width.max(height) as f32
    } else {
        1.0
    };

    let new_width = (width as f32 * scale) as u32;
    let new_height = (height as f32 * scale) as u32;
    if new_width == 0 || new_height == 0 {
        return None;
    }

    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, new_width, new_height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);

        if let Ok(mut writer) = encoder.write_header() {
            let mut resized = vec![0u8; (new_width * new_height * 4) as usize];
            for y in 0..new_height {
                for x in 0..new_width {
                    let src_x = (x as f32 / scale) as u32;
                    let src_y = (y as f32 / scale) as u32;
                    let src_idx = ((src_y * width + src_x) as usize) * 4;
                    let dst_idx = ((y * new_width + x) as usize) * 4;
                    if src_idx + 3 < data.len() && dst_idx + 3 < resized.len() {
                        resized[dst_idx..dst_idx + 4].copy_from_slice(&data[src_idx..src_idx + 4]);
                    }
                }
            }
            let _ = writer.write_image_data(&resized);
        }
    }

    if png_data.is_empty() {
        None
    } else {
        Some(png_data)
    }
}

// ─── Search Strategies ────────────────────────────────────────────

fn search_empty(conn: &rusqlite::Connection, filter: &str) -> Result<Vec<ClipResult>, String> {
    let sql = match filter {
        "all" => format!(
            "SELECT {} FROM clips ORDER BY {} LIMIT 50",
            SEL,
            list_order("")
        ),
        "pinned" => format!(
            "SELECT {} FROM clips WHERE pinned = 1 ORDER BY {} LIMIT 50",
            SEL,
            list_order("")
        ),
        f => format!(
            "SELECT {} FROM clips WHERE content_type = '{}' ORDER BY {} LIMIT 50",
            SEL,
            f,
            list_order("")
        ),
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    let rows = stmt.query_map([], row_to_clip).map_err(|e| e.to_string())?;
    for r in rows {
        results.push(r.map_err(|e| e.to_string())?);
    }
    Ok(results)
}

fn search_by_source_app(
    conn: &rusqlite::Connection,
    app_name: &str,
    filter: &str,
    temporal: &str,
) -> Result<Vec<ClipResult>, String> {
    let tc = if filter != "all" && filter != "pinned" {
        format!(" AND content_type = '{}'", filter)
    } else {
        String::new()
    };
    let pc = if filter == "pinned" {
        " AND pinned = 1"
    } else {
        ""
    };

    let sql = format!(
        "SELECT {} FROM clips WHERE LOWER(source_app) LIKE '%{}%'{}{}{}
         ORDER BY {} LIMIT 50",
        SEL,
        app_name.to_lowercase(),
        tc,
        pc,
        temporal,
        list_order("")
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    let rows = stmt.query_map([], row_to_clip).map_err(|e| e.to_string())?;
    for r in rows {
        results.push(r.map_err(|e| e.to_string())?);
    }
    Ok(results)
}

fn do_search_fts(
    conn: &rusqlite::Connection,
    query: &str,
    filter: &str,
    temporal: &str,
    sa: &str,
) -> Vec<ClipResult> {
    let fq = fts_q(query);
    if fq.is_empty() {
        return vec![];
    }
    let tc = if filter != "all" && filter != "pinned" {
        format!(" AND c.content_type = '{}'", filter)
    } else {
        String::new()
    };
    let pc = if filter == "pinned" {
        " AND c.pinned = 1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT c.id, c.content, c.content_type, c.source_app, c.created_at, c.pinned, c.content_highlighted, c.source_app_icon, c.ocr_text, c.image_thumbnail
         FROM clips_fts fts JOIN clips c ON c.id = fts.rowid
         WHERE clips_fts MATCH ?1{}{}{}{} ORDER BY fts.rank, {} LIMIT 50",
        tc,
        pc,
        temporal,
        sa,
        list_order("c.")
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = match stmt.query_map([&fq], row_to_clip) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    rows.filter_map(|r| r.ok()).collect()
}

fn do_search_vec(
    model: &crate::embed::EmbeddingModel,
    conn: &rusqlite::Connection,
    query: &str,
    filter: &str,
    temporal: &str,
    sa: &str,
) -> Vec<ClipResult> {
    let qv = match crate::embed::embed_query(model, query) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[Search] Embed failed: {}", e);
            return vec![];
        }
    };
    if qv.len() != 768 {
        return vec![];
    }
    let vb: Vec<u8> = qv.iter().flat_map(|f| f.to_le_bytes()).collect();
    let tc = if filter != "all" && filter != "pinned" {
        format!(" AND c.content_type = '{}'", filter)
    } else {
        String::new()
    };
    let pc = if filter == "pinned" {
        " AND c.pinned = 1"
    } else {
        ""
    };
    let sql = format!(
        "WITH knn AS (SELECT rowid, distance FROM clip_embeddings WHERE embedding MATCH ?1 AND k = 200)
         SELECT c.id, c.content, c.content_type, c.source_app, c.created_at, c.pinned, c.content_highlighted, c.source_app_icon, c.ocr_text, c.image_thumbnail
         FROM knn vec JOIN clips c ON c.id = vec.rowid
         WHERE 1=1{}{}{}{} ORDER BY vec.distance, {} LIMIT 50",
        tc,
        pc,
        temporal,
        sa,
        list_order("c.")
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[Search] Vec SQL: {}", e);
            return vec![];
        }
    };
    let rows = match stmt.query_map([vb], row_to_clip) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    rows.filter_map(|r| r.ok()).collect()
}

fn do_search_like(
    conn: &rusqlite::Connection,
    query: &str,
    filter: &str,
    temporal: &str,
    sa: &str,
) -> Vec<ClipResult> {
    let p = format!("%{}%", query);
    let tc = if filter != "all" && filter != "pinned" {
        format!(" AND content_type = '{}'", filter)
    } else {
        String::new()
    };
    let pc = if filter == "pinned" {
        " AND pinned = 1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT id, content, content_type, source_app, created_at, pinned, content_highlighted, source_app_icon, ocr_text, image_thumbnail
         FROM clips WHERE content LIKE ?1{}{}{}{} ORDER BY {} LIMIT 30",
        tc,
        pc,
        temporal,
        sa,
        list_order("")
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = match stmt.query_map([&p], row_to_clip) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    rows.filter_map(|r| r.ok()).collect()
}

// ─── Helpers ──────────────────────────────────────────────────────

const SEL: &str = "id, content, content_type, source_app, created_at, pinned, content_highlighted, source_app_icon, ocr_text, image_thumbnail";

fn list_order(prefix: &str) -> String {
    format!("{prefix}pinned DESC, {prefix}created_at DESC")
}

fn row_to_clip(r: &rusqlite::Row) -> rusqlite::Result<ClipResult> {
    let ic: Option<Vec<u8>> = r.get(7).unwrap_or(None);
    let thumbnail: Option<Vec<u8>> = r.get(9).unwrap_or(None);
    Ok(ClipResult {
        id: r.get(0)?,
        content: trunc(&r.get::<_, String>(1).unwrap_or_default()),
        content_type: r.get(2)?,
        source_app: r.get(3)?,
        created_at: r.get(4)?,
        pinned: r.get::<_, i64>(5)? != 0,
        source_app_icon: ic.filter(|b| !b.is_empty()).map(|b| b64(&b)),
        content_highlighted: r.get(6)?,
        ocr_text: r.get(8).unwrap_or(None),
        image_thumbnail: thumbnail.filter(|b| !b.is_empty()).map(|b| b64(&b)),
    })
}

fn fts_q(query: &str) -> String {
    let w: Vec<&str> = query.split_whitespace().collect();
    if w.is_empty() {
        return String::new();
    }
    if w.len() == 1 {
        return format!("{}*", w[0]);
    }
    w.iter()
        .enumerate()
        .map(|(i, w)| {
            if i == w.len() - 1 {
                format!("{}*", w)
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn trunc(s: &str) -> String {
    if s.len() > 500 {
        let e = s
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= 500)
            .last()
            .unwrap_or(0);
        format!("{}…", &s[..e])
    } else {
        s.to_string()
    }
}

fn b64(data: &[u8]) -> String {
    const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut r = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let (a, b, c2) = (
            c[0] as u32,
            if c.len() > 1 { c[1] as u32 } else { 0 },
            if c.len() > 2 { c[2] as u32 } else { 0 },
        );
        let t = (a << 16) | (b << 8) | c2;
        r.push(C[((t >> 18) & 0x3F) as usize] as char);
        r.push(C[((t >> 12) & 0x3F) as usize] as char);
        r.push(if c.len() > 1 {
            C[((t >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        r.push(if c.len() > 2 {
            C[(t & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    r
}

fn rrf(lists: Vec<Vec<ClipResult>>) -> Vec<ClipResult> {
    let k = 60.0;
    let mut sc: HashMap<i64, f64> = HashMap::new();
    let mut cm: HashMap<i64, ClipResult> = HashMap::new();
    for l in &lists {
        for (r, c) in l.iter().enumerate() {
            *sc.entry(c.id).or_insert(0.0) += 1.0 / (k + r as f64 + 1.0);
            cm.entry(c.id).or_insert_with(|| c.clone());
        }
    }
    let mut rv: Vec<(i64, f64)> = sc.into_iter().collect();
    rv.sort_by(|a, b| {
        let score_order = b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal);
        if score_order != std::cmp::Ordering::Equal {
            return score_order;
        }

        let left = cm.get(&a.0);
        let right = cm.get(&b.0);
        match (left, right) {
            (Some(left), Some(right)) => right
                .pinned
                .cmp(&left.pinned)
                .then_with(|| right.created_at.cmp(&left.created_at)),
            _ => std::cmp::Ordering::Equal,
        }
    });
    rv.into_iter()
        .filter_map(|(id, _)| cm.remove(&id))
        .collect()
}
