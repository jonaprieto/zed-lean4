; Tree-sitter highlighting used before (or without) Lean LSP semantic tokens.
; Keep generic captures before specific ones: for identical ranges, Zed gives
; later captures precedence.
; Comments.
(line_comment) @comment

(block_comment) @comment

(doc_comment) @comment.doc

(module_doc_comment) @comment.doc

; Literals.
(num_lit) @number

(scientific_lit) @number.float

(char_lit) @string @character

(str_lit) @string

(raw_string) @string.special

(interpolated_str) @string

(escape_sequence) @string.escape

; The grammar groups propositions and Boolean constructors in the same nodes.
([
  (true_const)
  (false_const)
] @boolean
  (#any-of? @boolean "true" "false"))

([
  (true_const)
  (false_const)
] @type.builtin
  (#any-of? @type.builtin "True" "False"))

[
  (type_const)
  (sort_const)
  (prop_const)
] @type.builtin

[
  (top_const)
  (bot_const)
  (empty_const)
] @constant.builtin

(sorry) @keyword

; Default identifier styling. More specific captures below override this.
(identifier) @variable

; Name literals contain identifiers, which need a more specific capture.
(name_lit
  name: (identifier) @string.special.symbol)

; Holes and metavariables.
[
  (hole)
  (synth_hole)
  (goal_const)
  (cdot)
] @variable.special

(named_hole
  (identifier) @variable.special)

; Binder names.
(binders
  (identifier) @variable.parameter)

(tuple_binder
  (identifier) @variable.parameter)

(anon_ctor_binder
  (identifier) @variable.parameter)

(explicit_binder
  name: (identifier) @variable.parameter)

(implicit_binder
  name: (identifier) @variable.parameter)

(strict_implicit_binder
  name: (identifier) @variable.parameter)

(instance_binder
  name: (identifier) @variable.parameter)

(paren
  arg_name: (identifier) @variable.parameter)

(tactic_case
  arg: (identifier) @variable.parameter)

; Definition styles fall back to variable in themes without a separate style.
(let
  name: (identifier) @variable.definition)

(have
  name: (identifier) @variable.definition)

(suffices
  name: (identifier) @variable.definition)

(by_cases
  name: (identifier) @variable.definition)

(if_then_else
  hyp: (identifier) @variable.definition)

; Declaration names.
(def
  name: (identifier) @function)

(theorem
  name: (identifier) @function)

(abbrev
  name: (identifier) @function)

(opaque
  name: (identifier) @function)

(constant
  name: (identifier) @function)

(axiom
  name: (identifier) @function)

(instance
  name: (identifier) @function)

(where_aux_def
  name: (identifier) @function)

(initialize
  name: (identifier) @function)

(alias_cmd
  name: (identifier) @function)

(structure
  name: (identifier) @type)

(inductive
  name: (identifier) @type)

(ctor
  name: (identifier) @constructor)

(ctor_alt
  name: (identifier) @constructor)

(field
  name: (identifier) @property)

; Namespaces and modules use optional theme-specific captures with standard
; fallbacks, resolved from right to left by Zed.
(namespace
  name: (identifier) @type @namespace)

(section
  name: (identifier) @type @namespace)

(end
  name: (identifier) @type @namespace)

(universe
  name: (identifier) @type.parameter)

(import
  name: (identifier) @type @module)

(open
  namespace: (identifier) @type @module)

(open
  only: (identifier) @function)

(open
  scoped: (identifier) @type @module)

(export
  namespace: (identifier) @type @module)

(export
  only: (identifier) @function)

; Attributes, projections, and calls.
(attributes
  name: (identifier) @attribute)

(proj
  field: (identifier) @property)

(app
  fn: (identifier) @function.call)

; A bare tactic is an invocation even when it has no arguments.
(by
  (identifier) @function.call)

(tactic_case
  body: (identifier) @function.call)

(tactic_case
  name: (identifier) @label)

; These do statements are identifiers in the static grammar. Restrict the
; textual fallback to statement positions, leaving ordinary names untouched.
((do_block
  (identifier) @keyword)
  (#any-of? @keyword "return" "break" "continue"))

((do_block
  (app
    fn: (identifier) @keyword))
  (#eq? @keyword "return"))

; Module system.
[
  (moduleTk)
  (prelude)
  (public)
  (meta)
  (all)
  (public_section)
  "import"
] @keyword.import

; Declarations and namespace commands.
[
  "namespace"
  "section"
  "end"
  "def"
  "theorem"
  "lemma"
  "example"
  "abbrev"
  "instance"
  "axiom"
  "opaque"
  "constant"
  "structure"
  "inductive"
  "class"
  "variable"
  "universe"
  "universes"
  "open"
  "export"
  "extends"
  "deriving"
  "where"
  "initialize"
  "builtin_initialize"
  "alias"
] @keyword

; Declaration modifiers.
[
  "noncomputable"
  "partial"
  "nonrec"
  "private"
  "protected"
  "unsafe"
  "mut"
  "scoped"
  "local"
  "rec"
] @keyword.modifier

; Commands and directives.
[
  "#check"
  "#check_failure"
  "#eval"
  "#exit"
  "#print"
  "#print_axioms"
  "#reduce"
  "#synth"
  "#version"
  "syntax"
  "macro"
  "macro_rules"
  "elab_rules"
  "declare_syntax_cat"
  "notation"
  "prefix"
  "postfix"
  "infix"
  "infixl"
  "infixr"
  "attribute"
  "set_option"
  "register_option"
  "register_builtin_option"
  "register_error_explanation"
] @keyword.directive

; Term and tactic keywords.
[
  "fun"
  "λ"
] @keyword.function

[
  "let"
  "letI"
  "have"
  "haveI"
  "show"
  "from"
  "suffices"
  "obtain"
  "by"
  "do"
  "for"
  "in"
  "case"
  "cases"
  "induction"
  "generalizing"
  "using"
  "by_cases"
] @keyword

[
  "if"
  "then"
  "else"
  "match"
  "with"
] @keyword.conditional

[
  "forall"
  "exists"
  "∀"
  "∃"
  "Π"
  "Σ"
  "Σ'"
] @keyword.operator

; Operators. Capturing the grammar's op fields covers the complete operator
; vocabulary, including Mathlib operators, without duplicating that list here.
(binary_op
  op: _ @operator)

(unary_op
  op: _ @operator)

(postfix_op
  op: _ @operator)

; Operators used outside binary/unary/postfix expression nodes.
[
  ":="
  ":"
  "=>"
  "↦"
  "←"
  "<-"
  "@"
] @operator

; Punctuation and delimiters.
[
  ","
  ";"
  "|"
  "."
] @punctuation.delimiter

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
  "⟨"
  "⟩"
  "⦃"
  "⦄"
  "{{"
  "}}"
  "⟪"
  "⟫"
  "⌈"
  "⌉"
  "⌊"
  "⌋"
  "⟦"
  "⟧"
  "‹"
  "›"
] @punctuation.bracket

"@[" @attribute

"#[" @punctuation.bracket

"..." @punctuation.special

; Quotation and interpolation introducers that are anonymous grammar tokens.
[
  "`"
  "``"
  "'"
  "''"
  "\""
  "s!\""
  "m!\""
] @string
