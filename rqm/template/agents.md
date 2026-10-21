# Feature: Codex and OpenCode <!-- rq-a70e0d9d -->

Besides Claude Code, a TopHat project can be used with Codex and OpenCode. Each is offered by the
`agents` Copier question; for each selected one, the template writes thin skill adapters, a
permission file with the deny and allow lists of `rqm/template/skills.md`, and the hook that runs
the container check before every prompt, and the image installs the agent. Skills, hard rules and
the worked example are the same for every agent; only these files differ.

## Copier Question <!-- rq-b9b84d13 -->

The `agents` multi-select (see `rqm/template/project.md`) also offers `codex` and `opencode`.

## Files <!-- rq-47e4462c -->

```text
.agents/skills/th-<name>/SKILL.md     managed   Codex adapter
.codex/rules/tophat.rules             managed   Codex command rules
.codex/hooks.json                     managed   Codex container check
.opencode/skills/th-<name>/SKILL.md   managed   OpenCode adapter
opencode.json                         managed   OpenCode permissions
.opencode/plugins/check-container.js  managed   OpenCode container check
```

- Each is written only when its agent is selected, and listed in
  `.tophat/managed/ownership-exceptions`. `opencode.json` and `.codex/hooks.json` are JSON and
  therefore marker-exempt.
- Adapters follow the adapter rules of `rqm/template/skills.md`. To ask the human, the Codex adapter
  names Codex's structured user input (or a concise textual question) and the OpenCode adapter
  OpenCode's question tool; Codex invokes other skills as `$th-<name>`, OpenCode as `/th-<name>`.
- Both read the agent entry point `AGENTS.md`.

## Permissions <!-- rq-6467b79d -->

| Agent | File | Mechanism |
|---|---|---|
| Codex | `.codex/rules/tophat.rules` | `prefix_rule` entries with `decision = "forbidden"` or `"allow"` |
| OpenCode | `opencode.json` | `permission.bash` entries `"<command> *": "deny"` or `"allow"` |

Each holds the same deny and allow lists as Claude Code's `.claude/settings.json` (see
`rqm/template/skills.md`).

## Image <!-- rq-ad167d22 -->

The agent layer of the image (see `rqm/template/container.md`) installs exactly the selected
agents, at the versions in `.tophat/managed/container/versions.yaml`, with self-updaters disabled.

## Assumptions <!-- rq-101734fd -->

- Codex reads project command rules from `.codex/rules/` and hooks from `.codex/hooks.json` in a
  trusted project; OpenCode reads `opencode.json` and `.opencode/plugins/` from the project root.

## Out of Scope <!-- rq-027991e8 -->

- Agents other than Claude Code, Codex and OpenCode.

## Gherkin Scenarios <!-- rq-bf79b510 -->

```gherkin
Feature: Codex and OpenCode

  @rq-1652b11e
  Scenario: Adapters are written for every selected agent
    Given the template is rendered with agents ["claude-code", "codex", "opencode"]
    Then .agents/skills and .opencode/skills each contain an adapter SKILL.md for each of the seven skills
    And the skill lint reports no "skill.adapter-missing"

  @rq-3ca62dea
  Scenario: No adapters for unselected agents
    When the template is rendered with agents ["codex"]
    Then .agents/skills/th-study/SKILL.md exists
    And no .claude/ or .opencode/ directory, CLAUDE.md or opencode.json exists

  @rq-a4edf228
  Scenario: Remote access and human-only commands are denied for every agent
    Given the template is rendered with agents ["claude-code", "codex", "opencode"]
    Then .codex/rules/tophat.rules forbids, and opencode.json denies, exactly the command prefixes .claude/settings.json denies
    And both allow exactly the commands .claude/settings.json allows

  @rq-d4406141
  Scenario: The container check runs before every prompt
    Given the template is rendered with agents ["codex", "opencode"]
    Then .codex/hooks.json and .opencode/plugins/check-container.js run .tophat/managed/hooks/check-container.sh

  @rq-f419a09b
  Scenario: Only selected agents are installed
    Given the project was rendered with agents ["codex"]
    When the image is built
    Then "codex --version" succeeds in the image
    And "claude" and "opencode" are not on PATH in the image
```
