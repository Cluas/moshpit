# Derive "<run>_guard": where the hotword run's output is an echo of the
# hotword list, take the plain run's output instead (what the engine does).
import json, sys, re
sys.path.insert(0, __import__('os').path.dirname(__import__('os').path.abspath(__file__)))
from utts import HOTWORDS
def is_echo(text, words):
    t = text.lower()
    hits = [w for w in words if w.lower() in t]
    if len(hits) < 3: return False
    covered = sum(len(re.sub(r'\s', '', w)) for w in hits)
    return covered >= 0.8 * len(re.sub(r'\s', '', text))
hw, plain, out = sys.argv[1:4]
p = {json.loads(l)['id']: json.loads(l) for l in open(plain)}
n = 0
with open(out, 'w') as f:
    for l in open(hw):
        d = json.loads(l)
        if is_echo(d['hyp'], HOTWORDS):
            n += 1
            d = dict(p[d['id']], sec=d['sec'] + p[d['id']]['sec'])
        f.write(json.dumps(d, ensure_ascii=False) + '\n')
print('replaced', n)
