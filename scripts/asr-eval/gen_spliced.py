# Condition B: CJK runs spoken by a zh voice, ASCII runs by an en voice,
# concatenated with 60 ms gaps. Same refs/terms as set A.
import json, subprocess, sys, os, re, wave, array
src = json.load(open(sys.argv[1])); out = sys.argv[2]
zh = ["Tingting", "Tingting", "Meijia"]
en = ["Samantha", "Eddy (English (US))"]
def synth(voice, text, path):
    subprocess.run(["say", "-v", voice, "--data-format=LEI16@16000", "--channels=1", "-o", path, text], check=True)
    w = wave.open(path); a = array.array('h', w.readframes(w.getnframes())); w.close(); return a
meta = []
for i, it in enumerate(src):
    runs = re.findall(r'[一-鿿，。]+|[^一-鿿，。]+', it['ref'])
    pcm = array.array('h')
    for j, r in enumerate(runs):
        r = r.strip()
        if not r: continue
        cjk = bool(re.search(r'[一-鿿]', r))
        v = zh[i % len(zh)] if cjk else en[i % len(en)]
        pcm.extend(synth(v, r, f"{out}/seg/{it['id']}_{j}.wav"))
        pcm.extend(array.array('h', [0]*960))
    path = f"{out}/wav/{it['id']}.wav"
    w = wave.open(path, 'wb'); w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000); w.writeframes(pcm.tobytes()); w.close()
    meta.append(dict(it, wav=path, voice="spliced"))
json.dump(meta, open(f"{out}/set.json", "w"), ensure_ascii=False, indent=1)
print(len(meta))
