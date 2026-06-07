; Each pattern captures @def for the full declaration so SymbolNode spans
; cover the body, enabling precise `enclosing_symbol` containment.

; Pattern 0: Function declarations
(function_declaration
  name: (identifier) @name) @def

; Pattern 1: Method declarations
(method_declaration
  name: (field_identifier) @name) @def

; Pattern 2: Struct type declarations
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (struct_type))) @def

; Pattern 3: Interface type declarations
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (interface_type))) @def

; Pattern 4: const declarations — one match per const_spec so grouped
; `const ( … )` blocks each yield a symbol.
(const_declaration
  (const_spec
    name: (identifier) @name) @def)

; Pattern 5: package-level var declarations — one match per var_spec.
(var_declaration
  (var_spec
    name: (identifier) @name) @def)

; Pattern 6: any other named type definition / alias (func, slice, map, named,
; `type X = Y`). Struct/interface forms already matched patterns 2/3; the Rust
; side skips those here via the captured @go_typekind so they are not duplicated.
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (_) @go_typekind)) @def

; Pattern 7: explicit type aliases `type X = Y` (a distinct grammar node).
(type_declaration
  (type_alias
    name: (type_identifier) @name)) @def
