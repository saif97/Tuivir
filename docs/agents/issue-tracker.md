# Issue tracker: GitHub

Issues and PRDs for this repository live in GitHub Issues at `saif97/Tuivir`. Use the `gh` CLI for all operations and pass `--repo saif97/Tuivir` when the repository cannot be inferred from the local Git remote.

## Conventions

- **Create an issue**: `gh issue create --repo saif97/Tuivir --title "..." --body "..."`
- **Read an issue**: `gh issue view --repo saif97/Tuivir <number> --comments`
- **List issues**: `gh issue list --repo saif97/Tuivir --state open --json number,title,body,labels,comments`
- **Comment on an issue**: `gh issue comment --repo saif97/Tuivir <number> --body "..."`
- **Apply or remove labels**: `gh issue edit --repo saif97/Tuivir <number> --add-label "..."` or `--remove-label "..."`
- **Close an issue**: `gh issue close --repo saif97/Tuivir <number> --comment "..."`

## Pull requests as a triage surface

**PRs as a request surface: no.**

GitHub shares one number space across issues and pull requests. When a bare reference such as `#42` is ambiguous, check whether it is a pull request before treating it as an issue.

## Skill operations

When a skill says to publish to the issue tracker, create a GitHub issue. When a skill says to fetch the relevant ticket, read the full issue body, comments, and labels.
