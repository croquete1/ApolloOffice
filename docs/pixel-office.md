# Pixel office and workflow

The active ApolloOffice interface keeps the pixel-art office in the main
canvas and adds the operational modules from the dashboard. The agent list
shows each worker's current status, assigned model and recent message.

## Modules

- **Queue:** shows waiting and recent work. The filter matches issue titles,
  repository names, issue numbers, work types, states and status details. The
  project selector filters task and PR lists together.
- **Pull requests:** shows repository PRs and their open, closed or merged
  state, with a 30-day merge chart.
- **Log:** shows recent worker and pipeline events from the live server state.
- **Pipeline:** counts all recorded work by stage, including RED/GREEN TDD,
  CI, independent review and human approval when staged mode is configured.
- **Productivity and resources:** show completed jobs by hour over the last 24
  hours and host telemetry only while the readings are recent.
- **Worker tiles:** show live agent status and let an operator pause or resume
  an agent; model settings remain available on the worker tile.

## Issue fixes and merging

Fix PR descriptions link back to their GitHub issues with GitHub closing
keywords. GitHub closes a linked issue when its PR is merged. Automatic merge
is enabled by default for new Apollo Office bug-fix PRs and can be disabled
per repository. It waits for verified fix and review evidence, a matching PR
head, and all GitHub checks to pass. Branch protection and merge queues still
apply; pending or failed checks leave the PR open.

The browser smoke check serves the real `ui/` files with a local synthetic
state. It does not contact production services or run agents.
