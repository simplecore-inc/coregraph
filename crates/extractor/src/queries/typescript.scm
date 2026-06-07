; Each pattern captures @name for the identifier token (used as the
; SymbolNode `name`) and @def for the full declaration range (used as
; the SymbolNode span_start/span_end). Using the full range means
; `enclosing_symbol` can strictly contain a reference inside a method
; body instead of falling back to "last preceding def" heuristics.

; Pattern 0: Class declarations
(class_declaration
  name: (type_identifier) @name) @def

; Pattern 1: Interface declarations
(interface_declaration
  name: (type_identifier) @name) @def

; Pattern 2: Function declarations
(function_declaration
  name: (identifier) @name) @def

; Pattern 3: Method definitions
(method_definition
  name: (property_identifier) @name) @def

; Pattern 4: Type aliases
(type_alias_declaration
  name: (type_identifier) @name) @def

; Pattern 5: Enum declarations
(enum_declaration
  name: (identifier) @name) @def

; Pattern 6: Enum variants (both `ADMIN` and `ADMIN = "admin"` forms)
(enum_declaration
  name: (identifier) @enum_name
  body: (enum_body
    (property_identifier) @name @def))

; Pattern 7: Enum variants via assignment (`ADMIN = ...`)
(enum_declaration
  name: (identifier) @enum_name
  body: (enum_body
    (enum_assignment
      name: (property_identifier) @name) @def))

; Pattern 8: Function-valued consts — the dominant modern TS/JS style.
;   `const x = () => {}` / `const x = function () {}`
;   `export const x = ((a) => ...) as X`     (parenthesized + type assertion)
;   `export const x = (() => ...) satisfies X` / `as unknown as X` (chained)
; The value is captured broadly (@fnbody) because the arrow can be wrapped in
; parens / `as` / `satisfies` / `!`; the Rust side unwraps those and keeps only
; declarators whose value really resolves to a function (see `unwraps_to_fn`).
; Without this, zustand-style public APIs (createStore, create, …) are
; invisible and nothing resolves to them. @def is the declarator so its span
; covers the body for strict-containment enclosing.
(variable_declarator
  name: (identifier) @name
  value: (_) @fnbody) @def

; Pattern 9: Class function-valued properties (`class C { handler = () => {} }`),
; same wrapper-unwrapping rule as pattern 8.
(public_field_definition
  name: (property_identifier) @name
  value: (_) @fnbody) @def
