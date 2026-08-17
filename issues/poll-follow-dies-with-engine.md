# `odm poll --follow` dies with the engine and nothing brings it back

From agent feedback (real session, 2026-08). AGENTS.md tells the agent to
park one standing `--follow` for the session so the user always has someone
listening. But when the engine stops, the poll exits 2 (`engine closed the
connection without responding`) and the standing command is gone — if the
user restarts the engine, nobody is listening and the viewer tells them so,
through no fault of theirs. With the engine down `odm say` fails too, so the
agent can't even explain via the viewer. The agent ended up wrapping the
poll in a shell retry loop waiting for `.odm/engine.sock` to reappear.

Ask: `--reconnect`, or make `--follow` reconnect by default — it is already
the "runs forever" mode, so exiting on engine death defeats its purpose.
Exiting on engine death stays right for plain `poll`.

Related prompt tweak (same session, different failure): the agent read the
"stay reachable" guidance and still defaulted to relaunching
`odm poll --timeout`; the user had to point out `--follow`. The `--follow`
sentence should lead the section rather than close it, and the CLI summary
block at the top of AGENTS.md could show `odm poll --follow` as the primary
form with `--timeout` as the fallback.
