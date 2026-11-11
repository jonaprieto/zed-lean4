; Declaration-sized motions and selections for Vim mode.
(declaration
  [
    (def)
    (theorem)
    (example)
    (abbrev)
    (opaque)
    (constant)
    (axiom)
    (instance)
  ]) @function.around

[
  (initialize)
  (where_aux_def)
] @function.around

; The body field also labels the anonymous := token, so use a named wildcard
; to select the value. Include trailing auxiliary definitions in the body.
(declaration
  (_
    body: (_) @function.inside
    (where_decls)? @function.inside))

; Equation-style declarations have match alternatives instead of a body field.
(declaration
  (_
    (match_alt) @function.inside
    [
      (match_alt)
      (line_comment)
      (block_comment)
      (doc_comment)
    ]* @function.inside
    (where_decls)? @function.inside))

; A structure-valued definition or instance has fields after where.
(declaration
  (_
    (where_struct
      (struct_field) @function.inside
      [
        (struct_field)
        (line_comment)
        (block_comment)
        (doc_comment)
      ]* @function.inside)
    (where_decls)? @function.inside))

(where_aux_def
  body: (_) @function.inside)

(where_aux_def
  (match_alt) @function.inside
  [
    (match_alt)
    (line_comment)
    (block_comment)
    (doc_comment)
  ]* @function.inside)

(initialize
  [
    ":="
    "←"
    "<-"
  ]
  (_)+ @function.inside)

(initialize
  !name
  [
    "initialize"
    "builtin_initialize"
  ]
  (_)+ @function.inside)

(declaration
  [
    (structure)
    (inductive)
  ]) @class.around

; Zed combines repeated captures from one match into the members' full range.
(structure
  "where"
  [
    (ctor)
    (field)
    (line_comment)
    (block_comment)
    (doc_comment)
  ]+ @class.inside)

(inductive
  (ctor_alt) @class.inside
  [
    (ctor_alt)
    (line_comment)
    (block_comment)
    (doc_comment)
  ]* @class.inside)

; namespace/section/end are flat sibling commands in this grammar, so their
; header nodes cannot provide an enclosing class object.
; A single-line object cannot accidentally span comments separated by blanks.
(line_comment) @comment.around

[
  (block_comment)
  (doc_comment)
  (module_doc_comment)
] @comment.around
