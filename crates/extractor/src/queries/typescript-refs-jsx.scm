; JSX element references — TSX grammar only.
;
; This query is appended to `typescript-refs.scm` ONLY for `.tsx` files, because
; `jsx_opening_element` / `jsx_self_closing_element` do not exist in the plain
; `typescript` grammar — compiling them against it fails the whole query and
; silently drops every `.ts` reference. Keeping JSX isolated here lets `.ts`
; files use the base query unchanged while `.tsx` files get JSX on top.
;
; Rendering a component in markup (`<Icon/>`, `<Panel>…</Panel>`) is a real use
; of it; without this, components used only in JSX (Storybook stories, page
; trees) are reported as dead code. Lowercase HTML tags (`<div>`) are captured
; too but resolve to no symbol, so they are inert. Emitted as a Call reference
; (rendering ≈ invoking the component). These are the LAST patterns, so their
; pattern indices come after every base pattern.
(jsx_opening_element
  name: (identifier) @call)
(jsx_self_closing_element
  name: (identifier) @call)
