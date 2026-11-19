"""Write tests/fixtures/pronunciation.json: what the Python app sends to the voice
for many texts, rules and filter choices. The Rust tests check they read text the
same way. Run from the repository root:  .venv/bin/python rust/tests/make_pronunciation_fixtures.py
"""
import json
from pathlib import Path

from textreader.pronunciation import FILTERS, apply, clean_filters, filter_example, new_rule

URL = "https://github.com/masonje/textReader/pull/6"
TEXTS = [
    URL, "jane.doe@example.com", "/home/jane/projects/textReader/reader.py",
    "6bb6c43f139d6d0111b6d152aaaf150bcfc26570", "4111111111111111",
    "~/github/textReader/reader.py", "../notes/todo.txt", r"C:\Users\jane\notes.txt",
    f"See {URL}.", f"(see {URL})", "https://en.wikipedia.org/wiki/Python_(programming_language)",
    "version 1.8.1 ships today", "call 555-1234 now", "the and/or choice", "a well-known example",
    "Visit www.example.com/a/b?x=1#top, then email bob_smith+news@mail.example.co.uk or open ./src/main.rs.",
    "Order 1Z999AA10123456784 shipped; ticket ABC-12345-XYZ-67890 and uuid 123e4567-e89b-12d3-a456-426614174000.",
    "Ünïcode café https://例え.jp/パス/ファイル.html and naïve /usr/lib/x86_64-linux-gnu/libmpv.so.2!",
    "Files: /etc/hosts, ./run.sh, ../up/one, ~/notes.md; also D:\\Games\\save1.dat.",
    "Short codes like A1B2C3 or 2026-11-19 stay; 0123456789ab gets read as a code.",
]

RULE_CASES = [
    ("Nguyen called.", [new_rule("Nguyen", "say", "win")], None),
    ("Run the SQL query", [new_rule("SQL", "spell")], None),
    ("See York[1], then go.", [new_rule("[1]", "skip")], None),
    ("a GIF file", [new_rule("GIF", "say", "jif")], None),
    ("GIFT shop", [new_rule("GIF", "say", "jif")], None),
    ("GIFT shop", [dict(new_rule("GIF", "say", "jif"), whole_word=False)], None),
    ("New York[1] is big", [new_rule("[1]", "skip")], None),
    ("Rx filled", [dict(new_rule("Rx", "say", "prescription"), match_case=True)], None),
    ("RX filled", [dict(new_rule("Rx", "say", "prescription"), match_case=True)], None),
    ("RX filled", [new_rule("Rx", "say", "prescription")], None),
    ("New York and New Jersey", [new_rule("New", "say", "knew"), new_rule("New York", "say", "the big apple")], None),
    ("GIF", [dict(new_rule("GIF", "say", "jif"), enabled=False)], None),
    ("GIF", [dict(new_rule("GIF", "say", "jif"), engine="Piper")], "Piper"),
    ("GIF", [dict(new_rule("GIF", "say", "jif"), engine="Piper")], "gTTS"),
    ("C++is fun & R&D, see [2] , or [3].", [new_rule("C++", "say", "see plus plus"), new_rule("&", "say", "and"),
                                            new_rule("[2]", "skip"), new_rule("[3]", "skip")], None),
    ("Café CAFÉ café", [new_rule("café", "spell")], None),
    (f"Open {URL} on github", [new_rule("github", "say", "git hub")], "filters"),
]

cases = []
# Each choice of each filter on its own (others "full"), plus the defaults
settings = [clean_filters({})] + [
    {k: (choice if k == kind else "full") for k in FILTERS}
    for kind, spec in FILTERS.items() for choice in spec["choices"]]
for text in TEXTS:
    for filters in settings:
        cases.append({"text": text, "rules": [], "engine": None, "filters": filters,
                      "expected": apply(text, [], filters=filters)[0]})
for text, rules, engine in RULE_CASES:
    filters = clean_filters({}) if engine == "filters" else None
    engine = None if engine == "filters" else engine
    cases.append({"text": text, "rules": rules, "engine": engine, "filters": filters,
                  "expected": apply(text, rules, engine, filters)[0]})
examples = {kind: {choice: filter_example(kind, choice) for choice in spec["choices"]}
            for kind, spec in FILTERS.items()}

out = Path(__file__).parent / "fixtures" / "pronunciation.json"
out.write_text(json.dumps({"cases": cases, "examples": examples}, ensure_ascii=False, indent=1))
print(f"wrote {len(cases)} cases to {out}")
