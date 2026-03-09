# Research: Agency Agents

This directory is an isolated research sandbox for exploring the
[agency-agents](https://github.com/msitarzewski/agency-agents) persona system.

**Scope**: experimentation and personal growth only. Nothing here should
influence ZeroClaw's production architecture or engineering protocol.

## What this is

A collection of 61 specialized AI agent personas organized into divisions:
- engineering, design, marketing, product, project-management
- spatial-computing, specialized, support, testing, strategy

Each persona is available as a slash command in `.claude/commands/`.

## How to use

Invoke a persona with its slash command:

```
/orchestrator       — pipeline coordinator across all agents
/backend-architect  — system design, APIs, data architecture
/growth-hacker      — user acquisition, experimentation, metrics
/ux-researcher      — user research, synthesis, validation
/security-engineer  — threat modeling, secure design
/reality-checker    — testing, evidence collection, QA
/frontend-developer — UI, components, web performance
/ai-engineer        — model integration, inference, fine-tuning
```

To exit a persona, just say "exit persona" or start a new conversation.

## Source

https://github.com/msitarzewski/agency-agents

Agent files live in `.claude/commands/`. Add more by copying the persona
content from the source repo into a new `.md` file in that directory.

## Isolation note

This `CLAUDE.md` is intentionally scoped to this subdirectory. The root
ZeroClaw `CLAUDE.md` engineering protocol remains unaffected when working
here. These personas are for research — they do not define ZeroClaw's
architecture, security posture, or engineering norms.
