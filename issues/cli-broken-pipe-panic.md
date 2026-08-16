# CLI panics on broken pipe

`odm docs search pattern | head` →
`panicked at library/std/src/io/stdio.rs: failed printing to stdout:
Broken pipe (os error 32)`.

Rust ignores SIGPIPE by default, so `println!` panics when the reader
closes early. Affects every printing command, but `docs` (incl. the `prompt` topic) is
the long outputs agents actually pipe through `head`/`grep`.

Fix options: restore `SIGPIPE=SIG_DFL` at startup in the `odm` binary
(one `unsafe libc` call, standard for CLIs), or catch the write error
and exit 0. Found 2026-07-29 while smoke-testing `odm docs`.
