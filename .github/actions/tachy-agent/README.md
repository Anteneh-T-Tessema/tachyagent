# Tachy Agent GitHub Action

Run Tachy AI agents in your CI/CD pipeline — code review, security scanning, test analysis, and more.

## Usage

```yaml
- name: Code Review
  uses: Anteneh-T-Tessema/tachyagent/.github/actions/tachy-agent@main
  with:
    template: code-reviewer
    prompt: "Review the changes in this PR for bugs, style issues, and security concerns."

- name: Security Scan
  uses: Anteneh-T-Tessema/tachyagent/.github/actions/tachy-agent@main
  with:
    template: security-scanner
    prompt: "Scan the codebase for security vulnerabilities."
    fail-on-error: "true"
```

## Inputs

| Input | Required | Default | Description |
|-------|----------|---------|-------------|
| `template` | yes | — | Agent template name |
| `prompt` | yes | — | What to ask the agent |
| `model` | no | `gemma4:26b` | LLM model |
| `version` | no | `latest` | Tachy version to install |
| `fail-on-error` | no | `true` | Fail step if agent fails |
| `max-iterations` | no | `16` | Max agent iterations |

## Outputs

| Output | Description |
|--------|-------------|
| `summary` | Agent output text |
| `success` | `true` or `false` |
| `iterations` | Number of iterations |
| `tool-invocations` | Number of tool calls |

## Example: PR Review Pipeline

```yaml
name: AI Review
on: [pull_request]

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: AI Code Review
        id: review
        uses: Anteneh-T-Tessema/tachyagent/.github/actions/tachy-agent@main
        with:
          template: code-reviewer
          prompt: "Review the diff for this PR. Focus on bugs and security."

      - name: Post Review Comment
        if: always()
        uses: actions/github-script@v7
        with:
          script: |
            github.rest.issues.createComment({
              owner: context.repo.owner,
              repo: context.repo.repo,
              issue_number: context.issue.number,
              body: `## AI Code Review\n\n${{ steps.review.outputs.summary }}`
            })
```
