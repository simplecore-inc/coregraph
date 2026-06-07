; Each pattern captures @def for the full declaration so SymbolNode spans
; cover the body, enabling precise `enclosing_symbol` containment.

; Pattern 0: Struct declarations
(struct_item
  name: (type_identifier) @name) @def

; Pattern 1: Enum declarations
(enum_item
  name: (type_identifier) @name) @def

; Pattern 2: Trait declarations
(trait_item
  name: (type_identifier) @name) @def

; Pattern 3: Function declarations (top-level)
(function_item
  name: (identifier) @name) @def

; Pattern 4: Methods in impl blocks
(impl_item
  body: (declaration_list
    (function_item
      name: (identifier) @name) @def))

; Pattern 5: Type alias declarations (`type X = Y;`)
(type_item
  name: (type_identifier) @name) @def
