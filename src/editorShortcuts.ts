export interface ShortcutTarget { closest(selector: string): unknown }

/** Let editing fields and focused controls handle their native keyboard actions. */
export function editorShortcutAllowed(key: string, target: ShortcutTarget | null, modalOpen: boolean): boolean {
  if (key === 'Tab') return false;
  if (modalOpen || target?.closest('input,textarea,select,[contenteditable="true"],[contenteditable]:not([contenteditable="false"]),dialog,[role="dialog"],.menu,[role="menu"],[role="menuitem"],button[aria-expanded="true"],[role="listbox"],[role="combobox"],[role="slider"],[role="spinbutton"]')) return false;
  if (/^Arrow|^(Home|End|PageUp|PageDown|Escape)$/.test(key)
    && target?.closest('button,a[href],summary,[role="button"],[role="menuitem"],[role="tab"],[role="tablist"],[role="checkbox"],[role="switch"],[role="separator"]')) return false;
  if (key === 'Space' || key === ' ' || key === 'Enter') {
    return !target?.closest('button,a[href],summary,[role="button"],[role="menuitem"],[role="tab"],[role="checkbox"],[role="switch"]');
  }
  return true;
}
