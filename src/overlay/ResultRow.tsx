import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Pin } from "lucide-react";
import { ClipResult } from "../hooks/useSearch";

interface ResultRowProps {
  result: ClipResult;
  isSelected: boolean;
  index: number;
  onClick: () => void;
  onDoubleClick: () => void;
}

function timeAgo(timestamp: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - timestamp;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
  return `${Math.floor(diff / 604800)}w ago`;
}

function getTypeBadge(type: string): string | null {
  switch (type) {
    case "url": return "URL";
    case "code": return "Code";
    case "image": return "IMG";
    default: return null;
  }
}

function getDomain(url: string): string {
  try { return new URL(url).hostname; } catch { return url; }
}

function cleanPreview(text: string): string {
  return text.replace(/[\r\n]+/g, " ").replace(/\s+/g, " ").trim();
}

function ResultRow({ result, isSelected, index, onClick, onDoubleClick }: ResultRowProps) {
  const badge = getTypeBadge(result.content_type);
  const preview = cleanPreview(result.content);
  const [imageThumbnail, setImageThumbnail] = useState<string | null>(result.image_thumbnail);

  useEffect(() => {
    let cancelled = false;

    setImageThumbnail(result.image_thumbnail ?? null);

    if (result.content_type === "image" && !result.image_thumbnail) {
      invoke<string | null>("get_image_thumbnail", { clipId: result.id })
        .then((data) => {
          if (!cancelled) {
            setImageThumbnail(data ?? null);
          }
        })
        .catch(() => {
          if (!cancelled) {
            setImageThumbnail(null);
          }
        });
    }

    return () => {
      cancelled = true;
    };
  }, [result.content_type, result.id, result.image_thumbnail]);

  return (
    <div
      data-no-drag
      className="flex items-center gap-3 px-4 cursor-pointer transition-colors duration-75 h-full rounded-lg mx-1"
      style={{
        background: isSelected ? "var(--surface-active)" : "transparent",
      }}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onMouseEnter={(e) => {
        if (!isSelected) e.currentTarget.style.background = "var(--surface-hover)";
      }}
      onMouseLeave={(e) => {
        if (!isSelected) e.currentTarget.style.background = "transparent";
      }}
    >
      {/* Number hint */}
      {index < 9 ? (
        <span className="row-number">{index + 1}</span>
      ) : (
        <span className="w-[14px] shrink-0" />
      )}

      {/* Content — always same height */}
      <div className="flex-1 min-w-0 overflow-hidden">
        {result.content_type === "url" ? (
          <div className="flex flex-col min-w-0">
            <span className="text-[13px] font-medium truncate" style={{ color: "var(--text-primary)" }}>{getDomain(result.content)}</span>
            <span className="text-[11px] truncate" style={{ color: "var(--text-tertiary)" }}>{result.content}</span>
          </div>
        ) : result.content_type === "code" && result.content_highlighted ? (
          <div
            className="text-[12px] leading-snug line-clamp-2 overflow-hidden"
            style={{ color: "var(--text-secondary)" }}
            dangerouslySetInnerHTML={{ __html: result.content_highlighted }}
          />
        ) : result.content_type === "image" ? (
          <div className="flex items-center gap-2.5 h-full">
            {imageThumbnail ? (
              <img
                src={`data:image/png;base64,${imageThumbnail}`}
                alt=""
                className="rounded-md h-[36px] w-[36px] object-cover shrink-0"
                style={{ border: "1px solid var(--border-default)" }}
              />
            ) : (
              <div
                className="h-[36px] w-[36px] rounded-md flex items-center justify-center text-[10px] shrink-0"
                style={{ background: "var(--surface-primary)", color: "var(--text-tertiary)" }}
              >
                IMG
              </div>
            )}
            <div className="flex flex-col min-w-0">
              <span className="text-[13px]" style={{ color: "var(--text-secondary)" }}>Image</span>
              {result.ocr_text && (
                <span className="text-[11px] truncate" style={{ color: "var(--text-tertiary)" }}>{cleanPreview(result.ocr_text)}</span>
              )}
            </div>
          </div>
        ) : (
          <span className="text-[13px] leading-snug line-clamp-2 overflow-hidden block" style={{ color: "var(--text-secondary)" }}>
            {preview}
          </span>
        )}
      </div>

      {/* Meta */}
      <div className="flex flex-col items-end shrink-0 gap-0.5">
        {result.source_app && (
          <span className="text-[11px] truncate max-w-[100px]" style={{ color: "var(--text-tertiary)" }}>{result.source_app}</span>
        )}
        <span className="text-[11px]" style={{ color: "var(--text-muted)" }}>{timeAgo(result.created_at)}</span>
      </div>

      {/* Badges + App icon on the right */}
      <div className="flex items-center gap-1.5 shrink-0">
        {badge && (
          <span className="text-[10px] px-1.5 py-0.5 rounded-full" style={{ background: "var(--badge-bg)", color: "var(--badge-text)" }}>{badge}</span>
        )}
        {result.pinned && (
          <span className="rounded-full p-1" style={{ background: "var(--accent-bg)", color: "var(--accent-text)" }}>
            <Pin size={10} strokeWidth={2.2} />
          </span>
        )}
        <div className="w-4 h-4 flex items-center justify-center">
          {result.source_app_icon ? (
            <img
              src={`data:image/png;base64,${result.source_app_icon}`}
              alt=""
              className="w-4 h-4 rounded-sm"
            />
          ) : (
            <div className="w-4 h-4 rounded" style={{ background: "var(--surface-primary)" }} />
          )}
        </div>
      </div>
    </div>
  );
}

export default ResultRow;
