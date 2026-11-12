; A run button beside each theorem and example opens the terminal infoview
; (the task tagged lean-infoview in tasks.json).
((declaration
  (theorem
    name: (identifier) @run))
  (#set! tag lean-infoview))

((declaration
  (example
    "example" @run))
  (#set! tag lean-infoview))
