# AGENTS.md (Runtime Instructions)

These instructions apply when running commands in Octovalve.

## Output Discipline
- Avoid commands that produce large outputs.
- Prefer scoped search and filtering to extract only the needed lines.
- Check size before large reads (e.g., wc -l, ls -lh).
- If output contains "[output truncated]", refine the command and retry.

## Recommended Patterns
- rg -n "pattern" path | head -n 200
- sed -n '1,200p' file
- rg --files -g '*.rs'
- wc -l file
