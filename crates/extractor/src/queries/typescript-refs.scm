; Calls
(call_expression
  function: (identifier) @call)

(call_expression
  function: (member_expression
    property: (property_identifier) @call))

(new_expression
  constructor: (identifier) @call)

; Imports
(import_statement
  source: (string) @import)

(import_specifier
  name: (identifier) @import)

(import_specifier
  alias: (identifier) @import)

; extends
(class_heritage
  (extends_clause
    value: (identifier) @extends))

; Type-position references — a TypeAlias / Interface / Class used only in a type
; annotation (`x: Foo`) or type argument (`Array<Foo>`) would otherwise be
; flagged as an orphan and collapse impact analysis. Emitted as a TypeUse
; reference (-> References edge).
(type_annotation (type_identifier) @type)
(type_arguments (type_identifier) @type)
(as_expression (type_identifier) @type)
(class_heritage
  (implements_clause (type_identifier) @type))

; Barrel re-exports — `export { Foo } from './foo'`. The re-exported symbol is
; consumed via the barrel (a real use), so without this it is a false orphan.
; The `source:` child scopes this to re-exports (not bare local `export { x }`)
; so it does not connect every exported symbol. Emitted as an Import reference.
; (Child order matches the parse tree: export_clause then source.)
(export_statement
  (export_clause
    (export_specifier
      name: (identifier) @import))
  source: (string))

; Interface heritage (`interface A extends Base`) and type-alias RHS
; (`type Alias = Target`) — precise type references (resolve by name), so they
; connect a Base/Target used only here without the noise of broad member capture.
(extends_type_clause
  type: (type_identifier) @type)
(type_alias_declaration
  value: (type_identifier) @type)

; Function passed by name as a default value — a parameter default
; (`function f(cb = noop)`) or a destructuring default (`const { is401 =
; defaultIs401 } = config`). The referenced function is genuinely used; without
; this, helpers only ever supplied as a default look like dead code. Emitted as
; a Call reference. (JSX patterns are TSX-grammar-only and live in a separate
; query appended for .tsx files — do not add them here, or the query fails to
; compile against the plain TypeScript grammar and every .ts ref is lost.)
(assignment_pattern
  right: (identifier) @call)
(object_assignment_pattern
  right: (identifier) @call)
; Function passed by name as a call argument (`subscribe(snapshot)`,
; `useSyncExternalStore(sub, getSnapshot)`). Only bare-identifier arguments are
; captured; member/call expressions are handled by the call patterns above.
; Locals and parameters are not graph symbols, so over-capture resolves to
; nothing, and ambiguous names are dropped by the resolver's uniqueness gate.
(arguments
  (identifier) @call)
; Function passed by name as a function-parameter default (`function f(cb =
; noop)`). The parameter's `value:` field holds the default; an identifier there
; references a real function/constant. (Destructuring defaults use the
; *_assignment_pattern nodes above; parameter defaults use this distinct node.)
(required_parameter
  value: (identifier) @call)
(optional_parameter
  value: (identifier) @call)

; Type identifiers referenced inside composite types the precise patterns above
; miss: a conditional type's constraint and both branches (`K extends Constraint
; ? Then : Else`), and intersection/union members (`A & B`, `A | B`). Each is a
; real type reference; without these the referenced types look like dead code.
; The conditional's `left` is the tested type param (a binding), so it is left
; uncaptured. Emitted as TypeUse (-> References edge).
(conditional_type
  right: (type_identifier) @type)
(conditional_type
  consequence: (type_identifier) @type)
(conditional_type
  alternative: (type_identifier) @type)
(intersection_type
  (type_identifier) @type)
(union_type
  (type_identifier) @type)

; Generic type name — `Box<Item>` references the type `Box`, not only its
; argument `Item` (which `type_arguments` already captures). Generic-heavy type
; machinery (`EntityHookFor<X>`, `EntityHasListRole<T>`) needs this edge or the
; referenced generic looks dead. Builtins (`Array<…>`, `Promise<…>`) resolve to
; no project symbol and are inert.
(generic_type
  name: (type_identifier) @type)

; Function passed by name through a value-level ternary — `cond ? onYes :
; emptyGet` (e.g. `useSyncExternalStore(p ? p.get : emptyGet, …)`). Each bare
; identifier branch is a real use. Member/call branches are handled by the call
; patterns above; locals resolve to nothing.
(ternary_expression
  consequence: (identifier) @call)
(ternary_expression
  alternative: (identifier) @call)

; Wildcard re-export — `export * from './mod'`. The `*` anonymous token (absent
; in `export { X } from` which has an export_clause, and in `export * as ns`
; which wraps a namespace_export) precisely scopes this to bare star re-exports.
; The captured `source:` string is the module specifier; resolution expands it
; to edges into every public symbol of the target file, so a symbol surfaced
; only through a barrel `export *` is not a false orphan. Emitted as ReexportAll.
(export_statement
  "*"
  source: (string) @reexport)

; Value-position reads of a (typically module-level) symbol — the dominant TS
; orphan false-positive class. A top-level const referenced only via subscript
; (`MODE_VARIANTS[mode]`) or member access (`authClient.session`, `SET.has(x)`)
; produced no edge and looked dead. Capturing the OBJECT identifier (never the
; property/index, which are bare names that would over-connect by fanout) emits
; a precise ValueRef → References edge. Only bare-identifier objects are taken;
; `a.b.c` / `foo().bar` resolve via their own inner nodes, and locals/builtins
; resolve to nothing. These are the LAST base patterns; the .tsx-only JSX
; patterns are appended after them, so their indices come last of all.
(subscript_expression
  object: (identifier) @ref)
(member_expression
  object: (identifier) @ref)
