; Pattern 0: call_expression — function call sites
(call_expression
  function: (identifier) @call)

(call_expression
  function: (scoped_identifier
    name: (identifier) @call))

(call_expression
  function: (field_expression
    field: (field_identifier) @call))

; Pattern 3: use_declaration — import statements
(use_declaration
  argument: (identifier) @import)

(use_declaration
  argument: (scoped_identifier
    name: (identifier) @import))

(scoped_use_list
  path: (identifier) @import)

(use_list
  (identifier) @import)

(use_list
  (scoped_identifier
    name: (identifier) @import))

; Pattern 8: impl Trait for Struct — Implements edge
(impl_item
  trait: (type_identifier) @impl_trait)

(impl_item
  trait: (scoped_type_identifier
    name: (type_identifier) @impl_trait))

; Pattern 10: type-position references — a struct/enum/trait used only as a
; field / param / return / generic type would otherwise be flagged orphan.
; Emitted as a TypeUse reference (-> References edge).
(type_identifier) @type
