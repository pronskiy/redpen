# Corpus — 20 real texts (SPEC step A3.3)

This is the one step nothing else can start without, and the one step an agent must not do
for you. **Invented texts are worse than no texts**: a model asked to write "a Slack message
with typical Russian-speaker errors" writes the errors it already knows how to find, the
prompt scores brilliantly, and the gate tells you nothing.

## What to collect

20 things **you already wrote**, in the last month or two, in English, without editing them
afterwards. Aim for the mix you actually hit the hotkey on:

- ~8 Slack / Telegram messages (the short, fast, unedited ones — these carry the most tells)
- ~6 emails
- ~4 post or PR / issue drafts
- ~2 longer things — a paragraph of docs, a conference abstract

Skew toward things you sent **quickly**. A blog post you edited four times has had the
foreignness polished out of it; a Slack reply typed in nine seconds has not, and that is
exactly the text redpen will be pointed at.

## How to save them

One file per text, raw text only:

```
docs/corpus/01-slack.md
docs/corpus/02-slack.md
docs/corpus/07-email.md
docs/corpus/15-post.md
```

**Paste it exactly as you sent it.** No fixing typos on the way in, no adding context, no
frontmatter, no explanation of what it was about. The app will only ever see a naked
selection, so the corpus has to be naked selections too — anything else evaluates a prompt
you are not shipping.

The number is just for ordering, the suffix is for your own bookkeeping: neither is sent to
the model.

## Privacy

`.gitignore` excludes `docs/corpus/*.md` and `results-*.md` — your real writing and the
critiques of it stay local while repo visibility is undecided (SPEC §7). If you later decide
the repo stays private, drop those lines.

## Then

```sh
evals/run.sh -n     # confirm it sees all 20, spend nothing
evals/run.sh        # run the prompt across them
```

and fill in the rating column in the generated `results-*.md`.
