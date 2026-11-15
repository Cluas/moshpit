"""Score ASR hypotheses against the eval set.

MER: edit distance over mixed tokens (each CJK char one token, each ASCII
[a-z0-9_]+ run one token), after lowercasing, Traditional->Simplified, and
Chinese-numeral->digit normalisation applied to BOTH sides.
Term hit (strict): case-insensitive, whitespace collapsed, spaces around
/ . _ - removed, then substring. Numbers must come out as digits.
"""
import json, re, sys, os
from opencc import OpenCC
t2s = OpenCC('t2s')

DIG = {'零':0,'〇':0,'一':1,'二':2,'两':2,'三':3,'四':4,'五':5,'六':6,'七':7,'八':8,'九':9}
UNIT = {'十':10,'百':100,'千':1000}
def cn_num(s):
    def conv(m):
        t = m.group(0)
        if len(t) == 1 and t in '一两':  # 一下/两个 stay words
            return t
        if not any(c in UNIT for c in t):
            return ''.join(str(DIG[c]) for c in t)
        total, cur = 0, 0
        for ch in t:
            if ch in DIG: cur = DIG[ch]
            elif ch in UNIT:
                total += (cur or 1) * UNIT[ch]; cur = 0
        return str(total + cur)
    return re.sub(r'[零〇一二两三四五六七八九十百千]{1,6}', conv, s)

def norm(s):
    s = t2s.convert(s).lower()
    s = cn_num(s)
    return s

# Hesitation sounds, dropped from both sides: ASCEND's transcripts keep them
# and a dictation engine should not. Only unambiguous ones (not 啊/呀, which
# are also sentence particles).
FILLERS = {'呃', '嗯', 'um', 'uh', 'uhm', 'hmm', 'mm', 'er', 'eh'}

def toks(s):
    s = norm(s)
    return [t for t in re.findall(r'[一-鿿]|[a-z0-9_]+', s) if t not in FILLERS]

def ed(a, b):
    prev = list(range(len(b)+1))
    for i, x in enumerate(a, 1):
        cur = [i] + [0]*len(b)
        for j, y in enumerate(b, 1):
            cur[j] = min(prev[j]+1, cur[j-1]+1, prev[j-1] + (x != y))
        prev = cur
    return prev[-1]

def strict(s):
    s = t2s.convert(s).lower()
    s = re.sub(r'\s+', ' ', s)
    s = re.sub(r'\s*([/._\-])\s*', r'\1', s)
    return s

def has_trad(s):
    return any(t2s.convert(c) != c for c in s if '一' <= c <= '鿿')

def score(set_path, hyp_path):
    items = {x['id']: x for x in json.load(open(set_path))}
    hyps = {}
    for line in open(hyp_path):
        line = line.strip()
        if line.startswith('{'):
            d = json.loads(line); hyps[d['id']] = d
    agg = {k: [0, 0] for k in ('all', 'mix', 'zh', 'en')}
    th = tt = 0
    sec = aud = 0.0
    trad = 0
    drops = 0
    misses = []
    for vid, it in items.items():
        h = hyps.get(vid, {'hyp': '', 'sec': 0, 'audio': 0})
        r, p = toks(it['ref']), toks(h['hyp'])
        e = ed(r, p)
        drops += len(p) < 0.5 * len(r)
        for k in ('all', it['kind']):
            agg[k][0] += e; agg[k][1] += len(r)
        hs = strict(h['hyp'])
        for t in it['terms']:
            tt += 1
            if strict(t) in hs: th += 1
            else: misses.append((vid, t))
        sec += h.get('sec', 0); aud += h.get('audio', 0)
        trad += has_trad(h['hyp'])
    out = {k: round(100*v[0]/max(v[1],1), 1) for k, v in agg.items()}
    out['term_hit'] = round(100*th/max(tt,1), 1)
    out['terms'] = f'{th}/{tt}'
    out['rtf'] = round(sec/aud, 3) if aud else None
    out['trad_outputs'] = trad
    out['dropouts'] = drops
    out['n'] = len(hyps)
    return out, misses

if __name__ == '__main__':
    set_path = sys.argv[1]
    for hp in sys.argv[2:]:
        o, m = score(set_path, hp)
        print(os.path.basename(hp), json.dumps(o, ensure_ascii=False))
        if os.environ.get('MISSES'):
            print('   misses:', m)
