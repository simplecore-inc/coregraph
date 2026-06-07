; Pattern 0-2: method invocations
(method_invocation
  name: (identifier) @call)

(method_invocation
  object: (identifier) @call
  name: (identifier))

(object_creation_expression
  type: (type_identifier) @call)

; Pattern 3-4: import declarations
(import_declaration
  (scoped_identifier
    name: (identifier) @import))

(import_declaration
  (identifier) @import)

; Pattern 5-6: class extends / implements
(superclass
  (type_identifier) @extends)

(super_interfaces
  (type_list
    (type_identifier) @impl))

; Pattern 7: type-position references — a class/interface/enum used only as a
; field/param/return/generic/cast type would otherwise be flagged orphan.
; Emitted as a TypeUse reference (-> References edge).
(type_identifier) @type

; Pattern 8-9: annotation type references — `@RestController`, `@GetMapping`,
; `@Bean`, `@Entity`, ... The annotation node sits inside the annotated symbol's
; declaration span, so this connects the symbol to the (usually external)
; annotation type. That edge prevents framework-registered entry points
; (Spring beans/controllers/entities, JUnit @Test, etc.) — which are invoked by
; the framework via reflection, not by repo code — from being false "dead code"
; orphans. Treated as an Import so the external-package fallback keeps the edge.
(marker_annotation
  name: (identifier) @import)

(annotation
  name: (identifier) @import)
