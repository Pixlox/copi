import { useState, useEffect, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Copy, Pin, PinOff, Trash2, X } from "lucide-react";
import { ClipResult } from "../hooks/useSearch";

export interface SheetAction {
  id: string;
  icon: ReactNode;
  label: string;
  shortcut: string;
  tone?: "default" | "danger";
}

interface ActionsSheetProps {
  clip: ClipResult;
  actions: SheetAction[];
  selectedIndex: number;
  onClose: () => void;
  onSelect: (index: number) => void;
  onActivate: (index: number) => void;
}

function previewText(clip: ClipResult): string {
  const source = clip.content_type === "image" ? clip.ocr_text || "Image clip" : clip.content;
  return source.replace(/[\r\n]+/g, " ").replace(/\s+/g, " ").trim();
}

function ActionButton({
  icon,
  label,
  shortcut,
  selected,
  tone = "default",
  onMouseEnter,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  shortcut: string;
  selected: boolean;
  tone?: "default" | "danger";
  onMouseEnter: () => void;
  onClick: () => void;
}) {
  const toneStyle = (() => {
    if (tone === "danger") {
      return selected
        ? { borderColor: "var(--danger-border)", background: "var(--danger-bg)", color: "var(--danger-text)" }
        : { borderColor: "var(--border-default)", background: "var(--surface-secondary)", color: "var(--danger-text)" };
    }

    return selected
      ? { borderColor: "var(--accent-border)", background: "var(--accent-bg)", color: "var(--text-primary)" }
      : { borderColor: "var(--border-default)", background: "var(--surface-secondary)", color: "var(--text-primary)" };
  })();

  return (
    <button
      type="button"
      data-no-drag
      onMouseEnter={onMouseEnter}
      onClick={onClick}
      className="flex w-full items-center justify-between rounded-[14px] border px-4 py-2.5 text-left transition-colors"
      style={toneStyle}
    >
      <span className="flex items-center gap-3">
        <span style={{ color: selected ? "var(--text-primary)" : "var(--text-secondary)" }}>{icon}</span>
        <span className="text-[13px]">{label}</span>
      </span>
      <span
        className="rounded-md px-2 py-0.5 text-[10px]"
        style={{ background: "var(--surface-primary)", color: "var(--text-tertiary)" }}
      >
        {shortcut}
      </span>
    </button>
  );
}

function ImagePreview({ clipId, thumbnail }: { clipId: number; thumbnail: string | null }) {
  const [preview, setPreview] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<string | null>("get_image_preview", { clipId, maxSize: 400 })
      .then((result) => {
        if (!cancelled) setPreview(result);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [clipId]);

  const imgSrc = preview
    ? `data:image/png;base64,${preview}`
    : thumbnail
      ? `data:image/png;base64,${thumbnail}`
      : null;

  if (!imgSrc) return null;

  return (
    <div className="flex justify-center">
      <img
        src={imgSrc}
        alt="Clipboard image"
        className="rounded-lg"
        style={{
          maxWidth: "100%",
          maxHeight: "180px",
          objectFit: "contain",
          border: "0.5px solid var(--border-default)",
        }}
      />
    </div>
  );
}

function ActionsSheet({
  clip,
  actions,
  selectedIndex,
  onClose,
  onSelect,
  onActivate,
}: ActionsSheetProps) {
  const isImage = clip.content_type === "image";

  return (
    <div className="absolute inset-0 z-30 flex items-end justify-end p-4" style={{ background: "rgba(0,0,0,0.08)" }}>
      <button type="button" className="absolute inset-0 cursor-default" onClick={onClose} />
      <div
        data-no-drag
        className="relative w-full max-w-[348px] rounded-[20px] border p-3 backdrop-blur-2xl"
        style={{
          background: "var(--overlay-bg)",
          borderColor: "var(--border-default)",
          boxShadow: "var(--overlay-shadow)",
        }}
      >
        {/* Header */}
        <div className="mb-3 rounded-[14px] p-3" style={{ background: "var(--surface-secondary)" }}>
          <div className="mb-2 flex items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <div className="mb-1 text-[11px] uppercase tracking-[0.10em]" style={{ color: "var(--text-tertiary)" }}>
                Actions
              </div>

              {/* Image preview for image clips */}
              {isImage && (
                <div className="mb-2">
                  <ImagePreview clipId={clip.id} thumbnail={clip.image_thumbnail} />
                </div>
              )}

              <div className="line-clamp-2 text-[13px]" style={{ color: "var(--text-primary)" }}>
                {previewText(clip) || "Untitled clip"}
              </div>
              <div className="mt-1.5 flex items-center gap-2 text-[11px]" style={{ color: "var(--text-tertiary)" }}>
                <span>{clip.source_app || "Unknown app"}</span>
                <span>·</span>
                <span>{clip.content_type}</span>
              </div>
            </div>
            <button
              type="button"
              data-no-drag
              onClick={onClose}
              className="rounded-full p-1.5 transition-colors"
              style={{ background: "var(--surface-primary)", color: "var(--text-secondary)" }}
            >
              <X size={14} />
            </button>
          </div>
          <div className="text-[11px]" style={{ color: "var(--text-tertiary)" }}>
            Use <span style={{ color: "var(--text-secondary)" }}>↑↓</span> to move,{" "}
            <span style={{ color: "var(--text-secondary)" }}>Enter</span> to confirm
          </div>
        </div>

        {/* Actions */}
        <div className="space-y-1.5">
          {actions.map((action, index) => (
            <ActionButton
              key={action.id}
              icon={action.icon}
              label={action.label}
              shortcut={action.shortcut}
              tone={action.tone}
              selected={index === selectedIndex}
              onMouseEnter={() => onSelect(index)}
              onClick={() => onActivate(index)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

export function buildSheetActions(clip: ClipResult): SheetAction[] {
  return [
    {
      id: "pin",
      icon: clip.pinned ? <PinOff size={16} /> : <Pin size={16} />,
      label: clip.pinned ? "Unpin Clip" : "Pin Clip",
      shortcut: "⌘P",
    },
    {
      id: "copy",
      icon: <Copy size={16} />,
      label: "Copy to Clipboard",
      shortcut: "⇧↵",
    },
    {
      id: "delete",
      icon: <Trash2 size={16} />,
      label: "Delete Entry",
      shortcut: "⌘D",
      tone: "danger",
    },
  ];
}

export default ActionsSheet;
