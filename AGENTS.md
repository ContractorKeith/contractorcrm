# Repository agent context

- Read `CLAUDE.md` as the canonical repository context.
- When `../dotfiles/projects.POLICY.md` exists, read it for shared project rules.
- Codex main sessions additionally read `../dotfiles/codex/ORCHESTRATION.md`.
- Grok Build main sessions additionally read `../dotfiles/grok/ORCHESTRATION.md`.
- Do not apply another runtime's model assignments.

<!-- kodade:kodmem-project:v1:start -->
## KödMem project context

When KödMCP tools are available, use the `kodmem-project` skill before planning,
when prior project knowledge is needed, and before a substantive handoff.
Repository files and the issue tracker remain implementation truth; KödMem holds
durable project context.
<!-- kodade:kodmem-project:v1:end -->
