// TopHat-managed adapter: refuse to work outside the project's dev container. `copier update`
// changes this file; put project conventions in AGENTS.md instead.

import { existsSync } from 'node:fs'
import { join } from 'node:path'

const message =
  'TopHat: OpenCode must run inside the project dev container. Start a session with "./th.sh" and run OpenCode there.'

const CHECK_SCRIPT = '.tophat/managed/hooks/check-container.sh'

// OpenCode derives `worktree` from git, so it is empty whenever git cannot resolve one. Locate the
// managed check from whichever project root OpenCode provides, falling back to the working
// directory, so the guardrail tracks the real container check rather than git state.
function locateCheck(roots) {
  for (const root of roots) {
    if (!root) continue
    const script = join(root, CHECK_SCRIPT)
    if (existsSync(script)) return script
  }
  return undefined
}

function rejectOutsideContainer({ directory, worktree }) {
  if (process.platform === 'win32') throw new Error(message)

  // Fail closed: with no managed check to consult, refuse rather than assume a container.
  const script = locateCheck([directory, worktree, process.cwd()])
  if (!script) throw new Error(message)

  const result = Bun.spawnSync(['bash', script])
  if (result.exitCode !== 0) throw new Error(message)
}

export const TopHatContainerCheck = async ({ directory, worktree }) => ({
  'chat.message': async () => rejectOutsideContainer({ directory, worktree }),
})
