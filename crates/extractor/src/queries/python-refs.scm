; Calls
(call
  function: (identifier) @call)

(call
  function: (attribute
    attribute: (identifier) @call))

; Imports
(import_statement
  name: (dotted_name) @import)

(import_from_statement
  name: (dotted_name) @import)

(import_from_statement
  module_name: (dotted_name) @import)

; extends — Python class bases
(class_definition
  superclasses: (argument_list
    (identifier) @extends))

; Type-hint references — a class used only in a parameter / return / variable
; annotation (`x: Foo`, `-> Foo`) would otherwise be flagged orphan. Emitted as
; a TypeUse reference (-> References edge).
(type (identifier) @type)
