; Annotation-only matches let Zed associate doc comments with the following
; item. Attributes and modifiers remain part of the declaration's item range.
(doc_comment) @annotation

(namespace
  "namespace" @context
  name: (identifier) @name) @item

(section
  "section" @context
  name: (identifier) @name) @item

(declaration
  (def
    "def" @context
    name: (identifier) @name)) @item

(declaration
  (theorem
    [
      "theorem"
      "lemma"
    ] @context
    name: (identifier) @name)) @item

(declaration
  (abbrev
    "abbrev" @context
    name: (identifier) @name)) @item

(declaration
  (opaque
    "opaque" @context
    name: (identifier) @name)) @item

(declaration
  (constant
    "constant" @context
    name: (identifier) @name)) @item

(declaration
  (axiom
    "axiom" @context
    name: (identifier) @name)) @item

(declaration
  (instance
    "instance" @context
    name: (identifier) @name)) @item

(declaration
  (instance
    !name
    "instance" @context
    type: (_) @name)) @item

(declaration
  (structure
    [
      "structure"
      "class"
    ] @context
    name: (identifier) @name)) @item

(declaration
  (inductive
    "class"? @context
    "inductive" @context
    name: (identifier) @name)) @item

(initialize
  [
    "initialize"
    "builtin_initialize"
  ] @context
  name: (identifier) @name) @item

(alias_cmd
  "alias" @context
  name: [
    (identifier)
    (anon_ctor)
  ] @name) @item

(where_aux_def
  name: (identifier) @name) @item

(field
  name: (identifier) @name) @item

(ctor
  name: (identifier) @name) @item

(ctor_alt
  name: (identifier) @name) @item
