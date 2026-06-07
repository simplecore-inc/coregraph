; Each pattern captures @def for the full declaration so SymbolNode spans
; cover the body (not just the identifier). `enclosing_symbol` then strictly
; contains refs inside method bodies instead of falling back to heuristics.

; Class declarations
(class_declaration
  name: (identifier) @name) @def

; Interface declarations
(interface_declaration
  name: (identifier) @name) @def

; Method declarations
(method_declaration
  name: (identifier) @name) @def

; Constructor declarations
(constructor_declaration
  name: (identifier) @name) @def

; Field declarations
(field_declaration
  declarator: (variable_declarator
    name: (identifier) @name)) @def

; Enum declarations
(enum_declaration
  name: (identifier) @name) @def

; Enum constants (individual variants: ACTIVE, PENDING, ...)
(enum_constant
  name: (identifier) @name) @def
