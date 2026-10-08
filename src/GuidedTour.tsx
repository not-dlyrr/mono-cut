import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, X } from 'lucide-react';

const steps = [
  { target: 'import', title: 'Bring in your media', text: 'Click Import to choose video, audio or images. You can also drop files into the editor. Try each control as you follow along.' },
  { target: 'preview', title: 'Preview before you cut', text: 'Select a file in Media to see the Clip preview. Timeline shows your edit. Play, scrub or step one frame at a time below the picture.' },
  { target: 'add', title: 'Add a clip to your edit', text: 'Select a media file, then click Add to Timeline. It starts at the playhead. Dragging media onto a track works too.' },
  { target: 'timeline', title: 'Trim and split', text: 'Drag either edge of a clip to trim it. Place the playhead and click Split to make a cut. Move clips directly; snapping helps line them up.' },
  { target: 'inspector', title: 'Make the clip your own', text: 'Select a timeline clip to adjust Basic, Audio, Speed and Color. Advanced mode also shows keyframes and the full editing workspace.' },
  { target: 'save', title: 'Keep your project', text: 'Click Save to create a .monocut project you can reopen. Autosave helps with recovery, but save a named project to keep your work.' },
  { target: 'export', title: 'Export your finished video', text: 'When your timeline is ready, choose Export. Set the size, frame rate and quality, then choose the video file. You can cancel while it encodes.' },
];
export default function GuidedTour({ step, setStep, onClose, onStep }: { step: number; setStep: (step: number) => void; onClose: () => void; onStep: (step: number) => void }) {
  const [anchor, setAnchor] = useState<DOMRect | null>(null), [position, setPosition] = useState({ left: 20, top: 70 }); const card = useRef<HTMLDivElement>(null), next = useRef<HTMLButtonElement>(null);
  useEffect(() => { next.current?.focus(); const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } }; document.addEventListener('keydown', key); return () => document.removeEventListener('keydown', key); }, [onClose]);
  useEffect(() => onStep(step), [step]);
  useLayoutEffect(() => {
    function locate() { const element = document.querySelector(`[data-tour="${steps[step].target}"]`); const r = element?.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) { setAnchor(null); setPosition({ left: Math.max(12, window.innerWidth / 2 - 170), top: 75 }); return; } setAnchor(r); const height = card.current?.offsetHeight || 214, width = Math.min(342, window.innerWidth - 24); const below = r.bottom + 12, above = r.top - height - 12; const top = below + height < window.innerHeight - 10 ? below : above > 10 ? above : Math.max(14, Math.min(window.innerHeight - height - 14, r.top + 12)); const left = Math.max(12, Math.min(window.innerWidth - width - 12, r.left + r.width / 2 - width / 2)); setPosition({ left, top }); }
    locate(); window.addEventListener('resize', locate); const timer = window.setTimeout(locate, 180); return () => { window.removeEventListener('resize', locate); clearTimeout(timer); };
  }, [step]);
  return <>{anchor && <div className="tour-highlight" style={{ left: anchor.left - 4, top: anchor.top - 4, width: anchor.width + 8, height: anchor.height + 8 }} aria-hidden="true" />}<div ref={card} className="tour-card glass" style={position} role="dialog" aria-modal="false" aria-labelledby="tour-title" aria-describedby="tour-description">
    <div className="tour-top"><span>Quick tour · {step + 1} of {steps.length}</span><button aria-label="Skip tutorial" onClick={onClose}><X size={15} /></button></div><h2 id="tour-title">{steps[step].title}</h2><p id="tour-description">{steps[step].text}</p>
    <div className="tour-actions"><button className="text-button" onClick={onClose}>Skip tour</button><div><button className="tour-back" aria-label="Previous step" disabled={step === 0} onClick={() => setStep(step - 1)}><ArrowLeft size={14} /></button><button ref={next} className="primary-button" aria-label={step === steps.length - 1 ? 'Finish tutorial' : 'Next step'} onClick={() => step === steps.length - 1 ? onClose() : setStep(step + 1)}>{step === steps.length - 1 ? 'Start editing' : 'Next'}<ArrowRight size={13} /></button></div></div>
  </div></>;
}
