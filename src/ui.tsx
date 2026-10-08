import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode, type ButtonHTMLAttributes } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';

export function IconButton({ label, children, active, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; active?: boolean }) {
  const { className, ...rest } = props;
  return <button className={`icon-button ${active ? 'active' : ''} ${className || ''}`} {...rest} aria-label={label} title={label}>{children}</button>;
}
export function Dialog({ title, children, onClose, wide }: { title: string; children: ReactNode; onClose: () => void; wide?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null), titleId = useId();
  useEffect(() => { const d = ref.current; d?.showModal(); return () => d?.close(); }, []);
  return <dialog ref={ref} aria-labelledby={titleId} className={wide ? 'modal wide' : 'modal'} onCancel={e => { e.preventDefault(); onClose(); }} onClick={e => { if (e.target === e.currentTarget) onClose(); }}>
    <header className="modal-header"><h2 id={titleId}>{title}</h2><IconButton label="Close dialog" onClick={onClose}><X size={18} /></IconButton></header>{children}
  </dialog>;
}
export function Menu({ label, children, icon }: { label: string; children: ReactNode; icon?: ReactNode }) {
  const [open, setOpen] = useState(false), [closing, setClosing] = useState(false); const ref = useRef<HTMLDivElement>(null), menuRef = useRef<HTMLDivElement>(null); const [position, setPosition] = useState({ left: 0, top: 0 });
  function close() { setClosing(true); window.setTimeout(() => { setOpen(false); setClosing(false); }, 160); }
  useLayoutEffect(() => { if (!open || !ref.current) return; const r = ref.current.getBoundingClientRect(), height = menuRef.current?.offsetHeight || 250; setPosition({ left: Math.min(r.left, window.innerWidth - 270), top: r.bottom + height + 12 > window.innerHeight ? Math.max(8, r.top - height - 6) : r.bottom + 6 }); }, [open]);
  useEffect(() => { if (!open) return; const pointer = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node) && !menuRef.current?.contains(e.target as Node)) close(); }; const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { close(); ref.current?.querySelector('button')?.focus(); } }; document.addEventListener('pointerdown', pointer); document.addEventListener('keydown', key); return () => { document.removeEventListener('pointerdown', pointer); document.removeEventListener('keydown', key); }; }, [open]);
  return <div ref={ref} className="menu-wrap"><button className="menu-trigger" aria-expanded={open} aria-haspopup="true" onClick={() => open ? close() : setOpen(true)}>{icon}{label}</button>{open && createPortal(<div ref={menuRef} className={`menu glass ${closing ? 'closing' : ''}`} style={position} onClick={e => { if ((e.target as HTMLElement).closest('[data-close-menu]')) close(); }}>{children}</div>, document.body)}</div>;
}
export function Field({ label, children }: { label: string; children: ReactNode }) { return <label className="field"><span>{label}</span>{children}</label>; }
export function NumberField({ label, value, onChange, min, max, step = 1, suffix }: { label: string; value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; suffix?: string }) {
  const [draft, setDraft] = useState(String(Number.isFinite(value) ? Number(value.toFixed(4)) : 0)), [editing, setEditing] = useState(false), cancel = useRef(false);
  useEffect(() => { if (!editing) setDraft(String(Number.isFinite(value) ? Number(value.toFixed(4)) : 0)); }, [value, editing]);
  function commit() { setEditing(false); if (cancel.current) { cancel.current = false; setDraft(String(value)); return; } const parsed = Number(draft); if (!draft.trim() || !Number.isFinite(parsed) || parsed === value) { setDraft(String(value)); return; } const bounded = Math.min(max ?? Infinity, Math.max(min ?? -Infinity, parsed)); setDraft(String(bounded)); if (bounded !== value) onChange(bounded); }
  return <Field label={label}><div className="number-wrap"><input type="number" value={draft} min={min} max={max} step={step} onFocus={() => setEditing(true)} onChange={e => setDraft(e.target.value)} onBlur={commit} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); e.currentTarget.blur(); } if (e.key === 'Escape') { e.preventDefault(); cancel.current = true; e.currentTarget.blur(); } }} />{suffix && <span>{suffix}</span>}</div></Field>;
}
export function Splitter({ direction, onMove, label }: { direction: 'horizontal' | 'vertical'; onMove: (delta: number) => void; label: string }) {
  const last = useRef(0);
  return <div className={`splitter ${direction}`} role="separator" aria-label={label} aria-orientation={direction === 'horizontal' ? 'vertical' : 'horizontal'} tabIndex={0}
    onKeyDown={e => { const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 12 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -12 : 0; if (d) { e.preventDefault(); onMove(d); } }}
    onPointerDown={e => { last.current = direction === 'horizontal' ? e.clientX : e.clientY; e.currentTarget.setPointerCapture(e.pointerId); }}
    onPointerMove={e => { if (!e.currentTarget.hasPointerCapture(e.pointerId)) return; const p = direction === 'horizontal' ? e.clientX : e.clientY; onMove(p - last.current); last.current = p; }}
    onPointerUp={e => e.currentTarget.releasePointerCapture(e.pointerId)} />;
}
