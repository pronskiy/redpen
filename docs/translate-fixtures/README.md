# Translate fixtures (Epic F, step F1.5)

Twelve short texts for the translate prompt: four each in English, Russian and Dutch, and in
each language two with an idiom a literal reading gets wrong and two plain ones. They are
written for the test, not taken from anyone's real messages, so they are committed.

The file name is the expectation:

- `en-*` must come out in Russian; `ru-*` and `nl-*` in English.
- `*-idiom-*` must get a `---` and at least one note; `*-plain-*` must get none.

Run them with `evals/translate.sh`. It checks both rules mechanically and writes every output
to a sheet for reading by eye.
