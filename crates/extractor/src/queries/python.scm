; Each pattern captures @def for the full declaration so SymbolNode spans
; cover the body, enabling precise `enclosing_symbol` containment.

; Pattern 0: Class definitions
(class_definition
  name: (identifier) @name) @def

; Pattern 1: Function definitions
(function_definition
  name: (identifier) @name) @def

; Pattern 2: Module-level assignments (module constants / singletons such as
; `codes = …`). Scoped to the module root so function-local bindings are not
; captured. Covers plain and annotated (`x: T = …`) assignments.
(module
  (expression_statement
    (assignment
      left: (identifier) @name) @def))

; Pattern 3: Class-level assignments (class attributes / fields).
(class_definition
  body: (block
    (expression_statement
      (assignment
        left: (identifier) @name) @def)))
