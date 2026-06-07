; Each pattern captures @def for the full declaration so SymbolNode spans
; cover the body, enabling precise `enclosing_symbol` containment.

; Pattern 0: Function declarations
(function_declaration
  name: (identifier) @name) @def

; Pattern 1: Class declarations
(class_declaration
  name: (identifier) @name) @def

; Pattern 2: Method definitions
(method_definition
  name: (property_identifier) @name) @def

; Pattern 3: Function-valued consts (`const x = () => {}`,
; `const x = function () {}`, `const x = (() => {})`). The value is captured
; broadly (@fnbody) because the arrow may be parenthesized; the Rust side
; unwraps it and keeps only declarators whose value resolves to a function.
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: (_) @fnbody)) @def
