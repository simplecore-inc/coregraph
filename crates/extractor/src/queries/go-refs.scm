; Calls
(call_expression
  function: (identifier) @call)

(call_expression
  function: (selector_expression
    field: (field_identifier) @call))

; Imports
(import_spec
  path: (interpreted_string_literal) @import)

; Type-position references — a struct/interface used only as a composite literal
; (`Group{...}` / `&Group{...}`), a field/param/return type, or a var type would
; otherwise be flagged orphan. Emitted as a TypeUse reference (-> References edge).
(type_identifier) @type
