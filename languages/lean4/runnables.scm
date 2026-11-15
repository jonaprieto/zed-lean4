; A run button on the first line of every command opens the terminal infoview
; (the task tagged lean-infoview in tasks.json). Comments and imports get none.
((module
  (_) @run)
  (#not-match? @run "^(--|/-|(public )?(meta )?import )")
  (#set! tag lean-infoview))
