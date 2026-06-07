; Calls
(call_expression
  function: (identifier) @call)

(call_expression
  function: (member_expression
    property: (property_identifier) @call))

(new_expression
  constructor: (identifier) @call)

; Imports
(import_specifier
  name: (identifier) @import)

(import_specifier
  alias: (identifier) @import)

; extends
(class_heritage
  (identifier) @extends)
