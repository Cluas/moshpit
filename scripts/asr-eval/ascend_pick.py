# Pick ~50 short Mandarin–English code-switched clips from ASCEND's test split
# (CC BY-SA 4.0, https://huggingface.co/datasets/CAiRE/ASCEND) for evaluation.
import io, json, re, sys, wave
import numpy as np, soundfile as sf, pyarrow.parquet as pq
srcs, out = sys.argv[1:-1], sys.argv[-1]
FILLERS = {'um','uh','uhm','hmm','mm','oh','ah','eh','er','yeah','ok','okay','yes','no','so','and','the'}
rows = [r for src in srcs for r in pq.read_table(src).to_pylist()]
def words(t): return re.findall(r"[A-Za-z][A-Za-z'+#.-]*", t)
def good(x):
    if x['language'] != 'mixed' or not (2.5 <= x['duration'] <= 8.0): return False
    t = x['transcription']
    if '[' in t or len(re.findall(r'[一-鿿]', t)) < 6: return False
    if re.search(r'\b[a-z] [a-z] [a-z]\b', t, re.I): return False   # spelled letters
    return any(len(w) >= 3 and w.lower() not in FILLERS for w in words(t))
cand = [x for x in rows if good(x)]
# Round-robin over speakers (technology topic first within each speaker),
# so no one voice dominates.
from collections import defaultdict
by = defaultdict(list)
for x in sorted(cand, key=lambda x: (x['topic'] != 'technology', x['id'])):
    by[x['original_speaker_id']].append(x)
print('candidate speakers', {k: len(v) for k, v in sorted(by.items())})
pick = []
while len(pick) < 50 and any(by.values()):
    for k in sorted(by):
        if by[k] and len(pick) < 50:
            pick.append(by[k].pop(0))
meta = []
for x in pick:
    a = x['audio']; data, sr = sf.read(io.BytesIO(a['bytes']), dtype='float32')
    if data.ndim > 1: data = data.mean(axis=1)
    assert sr == 16000, sr
    path = f"{out}/wav/a{x['id']}.wav"
    sf.write(path, data, 16000, subtype='PCM_16')
    terms = sorted({w for w in words(x['transcription']) if len(w) >= 3 and w.lower() not in FILLERS}, key=str.lower)
    meta.append({'id': 'a' + x['id'], 'kind': 'mix', 'ref': x['transcription'], 'terms': terms,
                 'voice': f"ascend:{x['original_speaker_id']}", 'topic': x['topic'], 'wav': path})
json.dump(meta, open(f'{out}/set.json', 'w'), ensure_ascii=False, indent=1)
print(len(cand), len(meta), round(sum(x['duration'] for x in pick), 1), 's',
      len({m['voice'] for m in meta}), 'speakers', sum(len(m['terms']) for m in meta), 'terms')
for m in meta[:8]: print(m['id'], m['topic'], m['ref'], m['terms'])
