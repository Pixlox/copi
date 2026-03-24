use arboard::{Clipboard, ImageData};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

use crate::{
    macos::{get_app_icon_png, get_frontmost_app_info, FrontmostApp},
    AppState,
};

pub async fn watch_clipboard(app: &tauri::AppHandle) {
    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[Clip] Failed to init: {}", e);
            return;
        }
    };

    let mut last_text_hash = String::new();
    let mut last_image_hash = String::new();
    let mut last_non_copi_app: Option<FrontmostApp> = None;

    loop {
        // Check if paused
        let paused = {
            let state = app.state::<AppState>();
            let running = *state.clipboard_watcher_running.lock().unwrap();
            !running
        };
        if paused {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        }

        let current_frontmost = get_frontmost_app_info();
        if let Some(frontmost) = current_frontmost.clone() {
            if !frontmost.is_copi() && !frontmost.is_empty() {
                last_non_copi_app = Some(frontmost);
            }
        }
        let source_app = current_frontmost
            .filter(|app| !app.is_copi() && !app.is_empty())
            .or_else(|| last_non_copi_app.clone())
            .unwrap_or_default();

        // ── Text clipboard ────────────────────────────────────────
        if let Ok(text) = clipboard.get_text() {
            let hash = compute_hash(&text);
            if hash != last_text_hash && !text.trim().is_empty() {
                last_text_hash = hash.clone();
                last_image_hash.clear();

                if !crate::privacy::should_capture(&text, app) {
                    continue;
                }

                let content_type = detect_content_type(&text, None);
                let highlighted = if content_type == "code" {
                    Some(highlight_code(&text))
                } else {
                    None
                };

                insert_clip(
                    app,
                    &text,
                    &hash,
                    &content_type,
                    &source_app,
                    highlighted.as_deref(),
                );
            }
        }

        // ── Image clipboard ───────────────────────────────────────
        let img_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match clipboard.get_image() {
                Ok(image_data) => {
                    let pixels = image_data.bytes.as_ref();
                    if pixels.is_empty() {
                        return;
                    }
                    let hash = compute_hash_bytes(pixels);
                    if hash == last_text_hash {
                        return;
                    }
                    if hash == last_image_hash {
                        return;
                    }
                    last_image_hash = hash.clone();

                    let thumbnail = image_to_thumbnail(&image_data);

                    // Run OCR on the image
                    let ocr_text = {
                        let state = app.state::<AppState>();
                        if let Some(ref ocr) = state.ocr_engine {
                            match ocr.recognize_text(
                                pixels,
                                image_data.width as u32,
                                image_data.height as u32,
                            ) {
                                Ok(text) if !text.trim().is_empty() => {
                                    eprintln!("[OCR] Recognized {} chars", text.len());
                                    Some(text)
                                }
                                Ok(_) => {
                                    eprintln!("[OCR] No text found in image");
                                    None
                                }
                                Err(e) => {
                                    eprintln!("[OCR] Failed: {}", e);
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    };

                    insert_image_clip(
                        app,
                        &image_data,
                        thumbnail.as_deref(),
                        &hash,
                        &source_app,
                        ocr_text.as_deref(),
                    );
                }
                Err(_) => {} // No image on clipboard — normal
            }
        }));

        if let Err(e) = img_result {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown panic".into());
            eprintln!("[Image] Processing failed: {}", msg);
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tauri::command]
pub async fn copy_to_clipboard(app: tauri::AppHandle, clip_id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let content_type: String = conn
        .query_row(
            "SELECT content_type FROM clips WHERE id = ?",
            [clip_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;

    if content_type == "image" {
        let (raw_bytes, width, height): (Vec<u8>, i64, i64) = conn
            .query_row(
                "SELECT image_data, image_width, image_height FROM clips WHERE id = ?",
                [clip_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| format!("Image data not found: {}", e))?;

        drop(conn);

        if raw_bytes.is_empty() {
            return Err("Image data is empty".to_string());
        }

        let image = ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Owned(raw_bytes),
        };
        clipboard
            .set_image(image)
            .map_err(|e| format!("Failed to set image: {}", e))?;
    } else {
        let content: String = conn
            .query_row("SELECT content FROM clips WHERE id = ?", [clip_id], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;

        drop(conn);

        clipboard.set_text(&content).map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────

fn compute_hash(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn compute_hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn detect_content_type(content: &str, _source_app: Option<&str>) -> String {
    let trimmed = content.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return "url".to_string();
    }
    let has_newlines = content.contains('\n');
    let code_indicators = [
        "{", "}", "=>", "function", "def ", "import ", "class ", "//", "/*", "#!", "fn ", "pub ",
        "impl ", "struct ", "enum ", "const ", "let ", "mut ",
    ];
    let has_code = code_indicators.iter().any(|&i| content.contains(i));
    if has_newlines && has_code {
        return "code".to_string();
    }
    "text".to_string()
}

fn highlight_code(code: &str) -> String {
    use syntect::easy::HighlightLines;
    use syntect::highlighting::ThemeSet;
    use syntect::html::styled_line_to_highlighted_html;
    use syntect::parsing::SyntaxSet;

    let ps = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();
    let syntax = ps
        .find_syntax_by_extension("txt")
        .unwrap_or_else(|| ps.find_syntax_plain_text());
    let theme = &ts.themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);
    let mut html = String::from("<pre style=\"margin:0\">");
    for line in code.lines() {
        if let Ok(regions) = h.highlight_line(line, &ps) {
            if let Ok(frag) =
                styled_line_to_highlighted_html(&regions, syntect::html::IncludeBackground::No)
            {
                html.push_str(&frag);
            }
        }
        html.push('\n');
    }
    html.push_str("</pre>");
    html
}

/// Fetch the app icon via AppKit and the on-disk cache.
fn fetch_app_icon(state: &tauri::State<'_, AppState>, source_app: &FrontmostApp) -> Vec<u8> {
    if source_app.name.is_empty() || source_app.is_copi() {
        return Vec::new();
    }

    let _ = state;
    get_app_icon_png(source_app).unwrap_or_default()
}

fn insert_clip(
    app: &tauri::AppHandle,
    content: &str,
    hash: &str,
    content_type: &str,
    source_app: &FrontmostApp,
    highlighted: Option<&str>,
) {
    let state = app.state::<AppState>();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let capped = if content.len() > 100_000 {
        &content[..content
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= 100_000)
            .last()
            .unwrap_or(0)]
    } else {
        content
    };

    let icon = fetch_app_icon(&state, source_app);
    let conn = state.db.lock().unwrap();

    let result = conn.execute(
        "INSERT INTO clips (content, content_hash, content_type, source_app, source_app_icon, content_highlighted, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(content_hash) DO UPDATE SET
            source_app = CASE
                WHEN excluded.source_app <> '' THEN excluded.source_app
                ELSE clips.source_app
            END,
            source_app_icon = CASE
                WHEN length(excluded.source_app_icon) > 0 THEN excluded.source_app_icon
                ELSE clips.source_app_icon
            END,
            content_highlighted = COALESCE(excluded.content_highlighted, clips.content_highlighted),
            created_at = excluded.created_at",
        rusqlite::params![capped, hash, content_type, source_app.name, icon, highlighted, now],
    );

    if result.is_ok() {
        let clip_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM clips WHERE content_hash = ?",
                [hash],
                |row| row.get(0),
            )
            .ok();
        drop(conn);
        if let Some(clip_id) = clip_id {
            let _ = state.clip_tx.try_send(clip_id);
        }
        let _ = app.emit("new-clip", ());
    }
}

fn insert_image_clip(
    app: &tauri::AppHandle,
    image_data: &ImageData,
    thumbnail: Option<&[u8]>,
    hash: &str,
    source_app: &FrontmostApp,
    ocr_text: Option<&str>,
) {
    let state = app.state::<AppState>();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let raw_bytes = image_data.bytes.as_ref();
    let width = image_data.width as i64;
    let height = image_data.height as i64;
    let thumb = thumbnail.unwrap_or(&[]);

    let icon = fetch_app_icon(&state, source_app);

    let conn = state.db.lock().unwrap();
    let result = conn.execute(
        "INSERT INTO clips (content, content_hash, content_type, source_app, source_app_icon, ocr_text, image_data, image_thumbnail, image_width, image_height, created_at)
         VALUES ('[Image]', ?1, 'image', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(content_hash) DO UPDATE SET
            source_app = CASE
                WHEN excluded.source_app <> '' THEN excluded.source_app
                ELSE clips.source_app
            END,
            source_app_icon = CASE
                WHEN length(excluded.source_app_icon) > 0 THEN excluded.source_app_icon
                ELSE clips.source_app_icon
            END,
            ocr_text = COALESCE(excluded.ocr_text, clips.ocr_text),
            image_data = COALESCE(excluded.image_data, clips.image_data),
            image_thumbnail = CASE
                WHEN length(excluded.image_thumbnail) > 0 THEN excluded.image_thumbnail
                ELSE clips.image_thumbnail
            END,
            image_width = CASE
                WHEN excluded.image_width > 0 THEN excluded.image_width
                ELSE clips.image_width
            END,
            image_height = CASE
                WHEN excluded.image_height > 0 THEN excluded.image_height
                ELSE clips.image_height
            END,
            created_at = excluded.created_at",
        rusqlite::params![hash, source_app.name, icon, ocr_text, raw_bytes, thumb, width, height, now],
    );

    if result.is_ok() {
        let clip_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM clips WHERE content_hash = ?",
                [hash],
                |row| row.get(0),
            )
            .ok();
        drop(conn);
        if let (Some(clip_id), true) = (clip_id, ocr_text.is_some()) {
            let _ = state.clip_tx.try_send(clip_id);
        }
        let _ = app.emit("new-clip", ());
    }
}

fn image_to_thumbnail(image_data: &ImageData) -> Option<Vec<u8>> {
    let width = image_data.width as u32;
    let height = image_data.height as u32;
    let bytes = image_data.bytes.as_ref();

    let scale = if width > 200 || height > 200 {
        200.0 / width.max(height) as f32
    } else {
        1.0
    };

    let new_width = (width as f32 * scale) as u32;
    let new_height = (height as f32 * scale) as u32;

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
                    if src_idx + 3 < bytes.len() && dst_idx + 3 < resized.len() {
                        resized[dst_idx..dst_idx + 4].copy_from_slice(&bytes[src_idx..src_idx + 4]);
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
