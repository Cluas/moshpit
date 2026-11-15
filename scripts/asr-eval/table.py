import json, os, sys, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
ROOT = os.environ.get('ASR_ROOT', '/tmp/asr-eval')
from score import score
R = f'{ROOT}/results'
rows = [
 ("Whisper large-v3 turbo 954MB, zh + primer (app today)", "whisper_zh_primer"),
 ("Whisper large-v3 turbo 954MB, auto-detect", "whisper_auto"),
 ("Qwen3-ASR 0.6B int8", "qwen3"),
 ("Qwen3-ASR 0.6B int8 + hotwords", "qwen3_hw"),
 ("Qwen3-ASR 0.6B int8 + hotwords + echo guard", "qwen3_hwg"),
 ("Qwen3-ASR 0.6B int4", "qwen3i4"),
 ("Qwen3-ASR 0.6B int4 + hotwords", "qwen3i4_hw"),
 ("**Qwen3-ASR 0.6B int4 + hotwords + echo guard (picked)**", "qwen3i4_hwg"),
 ("Fun-ASR-Nano int8", "funnano"),
 ("Fun-ASR-Nano int8 + hotwords", "funnano_hw"),
 ("SenseVoice-Small int8", "sensevoice"),
 ("FireRedASR2-AED int8", "firered2"),
 ("Apple SpeechTranscriber zh-CN (today's 1st)", "apple_speech"),
 ("Apple DictationTranscriber zh-CN, long (today's 2nd)", "apple_dictlong"),
 ("Apple DictationTranscriber zh-CN, short", "apple_dictshort"),
 ("Apple DictationTranscriber zh-CN, short + context", "apple_dictshort_ctx"),
]
def mem(name):
    name = {'qwen3i4_hwg': 'qwen3i4_hw', 'qwen3_hwg': 'qwen3_hw'}.get(name, name)
    try:
        t = open(f'{R}/{name}.err').read()
        m = re.search(r'(\d+)\s+peak memory footprint', t)
        return f"{int(m.group(1))/1e9:.2f}" if m else ''
    except FileNotFoundError: return ''
print("| model | A MER | A mix | A zh | A en | A terms | A drops | B MER | B mix | B en | B terms | B drops | RTF | peak GB |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
for label, n in rows:
    cells = [label]
    a = b = None
    if os.path.exists(f'{R}/{n}.jsonl'): a, _ = score(f'{ROOT}/eval/set.json', f'{R}/{n}.jsonl')
    if os.path.exists(f'{R}/B_{n}.jsonl'): b, _ = score(f'{ROOT}/evalB/set.json', f'{R}/B_{n}.jsonl')
    cells += [f"{a['all']}", f"{a['mix']}", f"{a['zh']}", f"{a['en']}", f"{a['term_hit']}", f"{a['dropouts']}"] if a else ['–']*6
    cells += [f"{b['all']}", f"{b['mix']}", f"{b['en']}", f"{b['term_hit']}", f"{b['dropouts']}"] if b else ['–']*5
    cells += [f"{a['rtf']}" if a else '–', mem(n)]
    print('| ' + ' | '.join(cells) + ' |')

# Real speech: ASCEND, 50 clips (scripts/asr-eval/ascend_pick.py).
REAL = [
 ("Whisper large-v3 turbo, zh + primer (app today)", "R_whisper_zh_primer"),
 ("Whisper large-v3 turbo, auto-detect", "R_whisper_auto"),
 ("Qwen3-ASR 0.6B int4 (picked)", "R_qwen3i4"),
 ("Qwen3-ASR 0.6B int4 + unrelated pane hotwords + echo guard", "R_qwen3i4_hwg"),
 ("Qwen3-ASR 0.6B int8", "R_qwen3"),
 ("Fun-ASR-Nano int8", "R_funnano"),
 ("SenseVoice-Small int8", "R_sensevoice"),
 ("Apple SpeechTranscriber zh-CN", "R_apple_speech"),
]
if os.path.exists(f'{ROOT}/ascend/set.json'):
    print()
    print("| model | MER | English words | drops | RTF |")
    print("|---|---|---|---|---|")
    for label, n in REAL:
        if not os.path.exists(f'{R}/{n}.jsonl'):
            continue
        r, _ = score(f'{ROOT}/ascend/set.json', f'{R}/{n}.jsonl')
        print(f"| {label} | {r['all']} | {r['term_hit']} ({r['terms']}) | {r['dropouts']} | {r['rtf']} |")
