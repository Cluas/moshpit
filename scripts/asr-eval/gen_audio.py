import json, subprocess, sys, os
sys.path.insert(0, os.path.dirname(__file__))
from utts import UTTS
out = sys.argv[1]
zh_voices = ["Tingting", "Tingting", "Meijia", "Eddy (Chinese (China mainland))", "Flo (Chinese (China mainland))"]
en_voices = ["Samantha", "Eddy (English (US))"]
meta = []
for i, (kind, text, terms) in enumerate(UTTS):
    vid = f"u{i:02d}"
    voice = (en_voices[i % 2] if kind == "en" else zh_voices[i % len(zh_voices)])
    wav = os.path.join(out, "wav", vid + ".wav")
    subprocess.run(["say", "-v", voice, "--data-format=LEI16@16000", "--channels=1", "-o", wav, text], check=True)
    meta.append({"id": vid, "kind": kind, "ref": text, "terms": terms, "voice": voice, "wav": wav})
json.dump(meta, open(os.path.join(out, "set.json"), "w"), ensure_ascii=False, indent=1)
print(len(meta))
