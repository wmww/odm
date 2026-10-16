# External agents: supported again, mostly headless

An agent ODM didn't spawn (autonomous, or the user's own terminal
harness) is a supported use case again. The target is mostly
headless, agent-only use. Pairs with plans/odm-init.md, which is how
such an agent starts a project.

## Decided (user, 2026-10-15)

- **No chat channel.** `say` stays gone, and `poll` doesn't come back
  in any form. The user talks to an external agent in its own harness,
  and the Agent panel is not supported alongside one.
- **No user-state.** No camera in `status`, no copy-context action in
  the viewer. A camera only means something at the moment the user acts
  in the viewer, and that moment only exists for the managed agent.
- **No push.** There's nothing to wake an idle external agent with;
  that was the harness-mechanics problem poll had.
- **Diagnostics ride responses**, for an agent that isn't getting
  them pushed.

## Diagnostics on responses

When some view slot or file is failing that the command didn't itself
build (its own build errors already come back as the error), add one
line to the response's `warnings`:
`"2 files failing (parts/wheel.js, parts/axle.js); odm status for the errors"`.

- **Only on change.** Report when the failing set (or a member's
  error) differs from what the engine last reported this way, not on
  every response. Same "value changes only" rule as the push. A file
  the agent is knowingly leaving broken costs one line, not one per
  command, and `status` always has the full current state.
- **Only when nothing is pushing.** Gate it on the engine having no
  live managed agent (the `AgentHost` the pusher serves: viewer, with
  an agent spawned). Then diagnostics reach an agent exactly one way:
  pushed to the managed agent, or carried in responses for everyone
  else. Headless never spawns an agent, so it always takes the
  response path, and a viewer with no agent started does too.
  Rejected: tagging requests from the managed agent (an env var the
  CLI forwards), which is new transport plumbing for a combination
  (external agent + live panel agent) we don't support, and which
  breaks if a harness's sandboxed shell scrubs the environment.
- **Engine host warnings** ("file watcher unavailable") take the same
  path, on change.
- The "last reported" state is engine-wide, not per client. With one
  agent per project that's the same thing. If there are several
  clients, whoever asks next hears about it, and `status` stays truthful.
- Every command carries it, `status` included. `status` already shows
  the failures in full, so there it would be redundant; skip it there.

## Prompt

`docs/prompts/cli.md` "Talking with the user" assumes the panel. Split
it into two short paragraphs in the one prompt (both kinds of agent
read the same AGENTS.md block):

- **Run from the viewer's Agent panel:** the current text, roughly.
- **Otherwise:** the user, if there is one, talks to you in your own
  harness, and you can't see the viewer. Builds you didn't trigger that
  break show up as a `warnings` line on your next command; `odm status`
  has the details.

Keep the added text to a few lines (agent-surface.md: the prompt is the
scarce resource). Alternative considered: send the panel paragraph
over ACP to the managed agent only, so AGENTS.md is harness-neutral.
Rejected for now: it's a second prompt-delivery path, for ~1 KB.

## Policy and docs

- design-decisions.md "The agent is managed, over ACP": add a dated
  amendment saying external agents are supported for headless/agent-only
  use, CLI only, with no chat, user-state or push, and the CLI is
  no longer free to change under them.
- architecture.md "Talking to the agent" and docs/cli.md "Talking with
  the user": drop "not a supported use case", and describe the
  response-path diagnostics and the gate.
- The `poll`/`say` redirect errors (odm-cli and engine
  `requests.rs::removed`) say "ODM runs the agent itself now". Reword
  for both audiences: "there is no `poll`: talk to the user in your own
  harness, or, from the viewer's Agent panel, in the conversation".

## Tests

- Engine: a failing file not targeted by the command means one
  `warnings` line, the same failure again means none, a changed error
  means one again. With a managed agent attached (the agent tests'
  fake host), none.
- e2e: headless, break `parts/x.js` by hand, then `odm inspect` on
  root carries the warning.

## Not in scope

Selection for external agents beyond what `status` already has, any
viewer UI, and the panel+external combination.
