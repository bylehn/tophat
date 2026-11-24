# Feature: Codex, OpenCode and Vibe <!-- rq-a70e0d9d -->

Besides Claude Code, a TopHat project can be used with Codex, OpenCode and Mistral's Vibe. Each is
offered by the `agents` Copier question; for each selected one, the template writes thin skill
adapters, a permission file with the deny and allow lists of `rqm/template/skills.md`, and the hook
that runs the container check before every prompt, and the image installs the agent. Skills, hard
rules and the worked example are the same for every agent; only these files differ.

## Copier Question <!-- rq-b9b84d13 -->

The `agents` multi-select (see `rqm/template/project.md`) also offers `codex`, `opencode` and
`vibe`.

## Files <!-- rq-47e4462c -->

```text
.agents/skills/th-<name>/SKILL.md     managed   Codex adapter
.codex/rules/tophat.rules             managed   Codex command rules
.codex/hooks.json                     managed   Codex container check
.opencode/skills/th-<name>/SKILL.md   managed   OpenCode adapter
opencode.json                         managed   OpenCode permissions
.opencode/plugins/check-container.js  managed   OpenCode container check
.vibe/skills/th-<name>/SKILL.md       managed   Vibe adapter
.vibe/config.toml                     managed   Vibe permissions and container check
```

- Each is written only when its agent is selected, and listed in
  `.tophat/managed/ownership-exceptions`. `opencode.json` and `.codex/hooks.json` are JSON and
  therefore marker-exempt; `.vibe/config.toml` carries the marker in a TOML comment.
- Adapters follow the adapter rules of `rqm/template/skills.md`. To ask the human, the Codex adapter
  names Codex's structured user input (or a concise textual question), the OpenCode adapter
  OpenCode's question tool, and the Vibe adapter a concise textual question; Codex invokes other
  skills as `$th-<name>`, OpenCode and Vibe as `/th-<name>`.
- All three read the agent entry point `AGENTS.md`.

## Permissions <!-- rq-6467b79d -->

| Agent | File | Mechanism |
|---|---|---|
| Codex | `.codex/rules/tophat.rules` | `prefix_rule` entries with `decision = "forbidden"` or `"allow"` |
| OpenCode | `opencode.json` | `permission.bash` entries `"<command> *": "deny"` or `"allow"` |
| Vibe | `.vibe/config.toml` | `permissions` `deny` and `allow` lists with `"bash: <command> *"` entries |

Each holds the same deny and allow lists as Claude Code's `.claude/settings.json` (see
`rqm/template/skills.md`).

## Image <!-- rq-ad167d22 -->

The agent layer of the image (see `rqm/template/container.md`) installs exactly the selected
agents, at the versions in `.tophat/managed/container/versions.yaml`, with self-updaters disabled.

## Assumptions <!-- rq-101734fd -->

- Codex reads project command rules from `.codex/rules/` and hooks from `.codex/hooks.json` in a
  trusted project; OpenCode reads `opencode.json` and `.opencode/plugins/` from the project root;
  Vibe reads skills from `.vibe/skills/`, permissions and hooks from `.vibe/config.toml` in a
  trusted project, and user-level configuration from `~/.vibe/`.

## Out of Scope <!-- rq-027991e8 -->

- Agents other than Claude Code, Codex, OpenCode and Vibe.

## Gherkin Scenarios <!-- rq-bf79b510 -->

```gherkin
Feature: Codex, OpenCode and Vibe

  @rq-1652b11e
  Scenario: Adapters are written for every selected agent
    Given the template is rendered with agents ["claude-code", "codex", "opencode", "vibe"]
    Then .agents/skills, .opencode/skills and .vibe/skills each contain an adapter SKILL.md for each of the eight skills
    And the skill lint reports no "skill.adapter-missing"

  @rq-3ca62dea
  Scenario: No adapters for unselected agents
    When the template is rendered with agents ["vibe"]
    Then .vibe/skills/th-study/SKILL.md exists
    And no .claude/, .agents/, .codex/ or .opencode/ directory, CLAUDE.md or opencode.json exists

  @rq-a4edf228
  Scenario: Remote access and human-only commands are denied for every agent
    Given the template is rendered with agents ["claude-code", "codex", "opencode", "vibe"]
    Then .codex/rules/tophat.rules forbids, opencode.json denies, and .vibe/config.toml denies exactly the command prefixes .claude/settings.json denies
    And all three allow exactly the commands .claude/settings.json allows

  @rq-d4406141
  Scenario: The container check runs before every prompt
    Given the template is rendered with agents ["codex", "opencode", "vibe"]
    Then .codex/hooks.json, .opencode/plugins/check-container.js and the hook registered in .vibe/config.toml run .tophat/managed/hooks/check-container.sh

  @rq-f419a09b
  Scenario: Only selected agents are installed
    Given the project was rendered with agents ["vibe"]
    When the image is built
    Then "vibe --version" succeeds in the image
    And "claude", "codex" and "opencode" are not on PATH in the image

  @rq-72d4d9f0
  Scenario: The Vibe adapter names Vibe's own mechanisms
    Given the template is rendered with agents ["vibe"]
    Then every .vibe/skills adapter tells the agent to ask the human a concise textual question
    And tells it to invoke other skills as "/th-<name>"
```
