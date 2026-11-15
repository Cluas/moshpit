"""Run one sherpa-onnx offline model over the eval set; one JSON line per file.
usage: run_sherpa.py <kind> <model_dir> <set.json> [--hotwords] [--threads N] [--lang L]
"""
import json, os, sys, time, resource, argparse
import numpy as np, soundfile as sf
import sherpa_onnx
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from utts import HOTWORDS

ap = argparse.ArgumentParser()
ap.add_argument('kind'); ap.add_argument('model'); ap.add_argument('set')
ap.add_argument('--hotwords', action='store_true')
ap.add_argument('--threads', type=int, default=2)
ap.add_argument('--lang', default='')
ap.add_argument('--quant', default='int8')
a = ap.parse_args()
d = a.model
hw = ','.join(HOTWORDS) if a.hotwords else ''
t0 = time.time()
R = sherpa_onnx.OfflineRecognizer
if a.kind == 'sensevoice':
    rec = R.from_sense_voice(model=f'{d}/model.int8.onnx', tokens=f'{d}/tokens.txt',
                             num_threads=a.threads, use_itn=True, language=a.lang or 'auto')
elif a.kind == 'qwen3':
    rec = R.from_qwen3_asr(conv_frontend=f'{d}/conv_frontend.onnx', encoder=f'{d}/encoder.{a.quant}.onnx',
                           decoder=f'{d}/decoder.{a.quant}.onnx', tokenizer=f'{d}/tokenizer',
                           num_threads=a.threads, hotwords=hw)
elif a.kind == 'funasr_nano':
    rec = R.from_funasr_nano(encoder_adaptor=f'{d}/encoder_adaptor.int8.onnx', llm=f'{d}/llm.int8.onnx',
                             embedding=f'{d}/embedding.int8.onnx', tokenizer=f'{d}/Qwen3-0.6B',
                             num_threads=a.threads, hotwords=hw, language=a.lang)
elif a.kind == 'firered2':
    rec = R.from_fire_red_asr(encoder=f'{d}/encoder.int8.onnx', decoder=f'{d}/decoder.int8.onnx',
                              tokens=f'{d}/tokens.txt', num_threads=a.threads)
else:
    raise SystemExit('unknown kind')
load = time.time() - t0
items = json.load(open(a.set))
def run(wav):
    x, sr = sf.read(wav, dtype='float32')
    s = rec.create_stream()
    s.accept_waveform(sr, x)
    t = time.time(); rec.decode_stream(s); dt = time.time() - t
    return s.result.text, dt, len(x) / sr
run(items[0]['wav'])  # warm-up
for it in items:
    text, dt, dur = run(it['wav'])
    print(json.dumps({'id': it['id'], 'hyp': text, 'sec': dt, 'audio': dur}, ensure_ascii=False), flush=True)
print(f'load={load:.2f}s peak_rss_mb={resource.getrusage(resource.RUSAGE_SELF).ru_maxrss/1048576:.0f}', file=sys.stderr)
