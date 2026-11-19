"""Write tests/fixtures/word_timing.json: the Python app's word timelines for sample
texts, audio lengths and detected pauses. The Rust tests check they come out the same.
Run from the repository root:  .venv/bin/python rust/tests/make_timing_fixtures.py
"""
import json
from pathlib import Path

from textreader.pronunciation import apply, new_rule, spoken_words
from textreader.word_timing import build_timeline, tokenize

LONG = ("The quick brown fox jumps over the lazy dog. It was not amused; in fact, it barked twice!\n"
        "Then — after a pause — everyone (even the cat) went home. Version 1.8.1 shipped in 2026… "
        "\"Really?\" she asked. 'Yes,' he said.")
TEXTS = ["Hello.", "Hello there, world!", LONG, "One\ntwo\n\nthree four.", "Café naïve—déjà vu: ünïcode words."]
SILENCES = [
    [],
    [[0.0, 0.3]],
    [[0.0, 0.25], [2.1, 2.5], [4.0, 4.3], [9.5, None]],
    [[0.0, 0.2], [1.2, 1.5], [3.0, 3.6], [5.2, 5.4], [7.0, 7.5], [8.8, 9.1], [11.0, None]],
    [[0.02, 0.1], [0.5, 0.6], [6.0, 6.4], [11.95, 12.0]],
    [[0.0, 20.0]],  # silent all the way: falls back to an even spread
]
RULES = [new_rule("fox", "say", "red fox"), new_rule("1.8.1", "say", "one eight one"), new_rule("(even the cat)", "skip")]

cases = []
for text in TEXTS:
    spans = tokenize(text)
    for duration in (1.0, 12.0):
        for silences in SILENCES:
            sil = [tuple(s) for s in silences]
            cases.append({"text": text, "duration": duration, "silences": silences, "spoken": None,
                          "timeline": build_timeline(text, duration, sil, spans)})
            _, edits = apply(text, RULES)
            if edits:
                spoken = spoken_words(text, spans, edits)
                cases.append({"text": text, "duration": duration, "silences": silences, "spoken": spoken,
                              "timeline": build_timeline(text, duration, sil, spans, spoken)})
out = Path(__file__).parent / "fixtures" / "word_timing.json"
out.write_text(json.dumps(cases, ensure_ascii=False, indent=1))
print(f"{len(cases)} cases -> {out}")
