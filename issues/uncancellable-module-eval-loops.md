# Infinite JS loops at module top level are uncancellable

Cancellation covers kernel ops (CancelToken, ~tens of ms) and build()
execution (TerminateExecution via isolate handles). But isolate handles are
registered only *after* module evaluation (guarding rusty_v8 #830:
terminating during ES module evaluation can crash V8). So
`while(true){}` at a doohickey's top level — outside any function — hangs
that worker until the engine dies.

Options: (a) watchdog thread that terminates module eval anyway after a
generous timeout, accepting the (rare, possibly-fixed-upstream) #830 crash
risk on that path; (b) check whether newer rusty_v8 fixed #830 and register
handles before eval; (c) preparse: reject doohickeys with top-level
statements beyond imports/consts (too restrictive?). Revisit when it bites.
