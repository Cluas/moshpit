// Mirrors Qwen3ASRModel.swift's C configuration; decodes one 16 kHz mono WAV
// once with the given hotwords (or none). For peak-memory measurement.
#include "sherpa-onnx/c-api/c-api.h"
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
  const char *dir = argv[1], *wav = argv[2], *hot = argc > 3 ? argv[3] : "";
  char fe[512], en[512], de[512], tk[512];
  snprintf(fe, 512, "%s/conv_frontend.onnx", dir); snprintf(en, 512, "%s/encoder.int4.onnx", dir);
  snprintf(de, 512, "%s/decoder.int4.onnx", dir); snprintf(tk, 512, "%s/tokenizer", dir);
  SherpaOnnxOfflineRecognizerConfig c; memset(&c, 0, sizeof c);
  c.feat_config.sample_rate = 16000; c.feat_config.feature_dim = 128;
  c.model_config.qwen3_asr.conv_frontend = fe; c.model_config.qwen3_asr.encoder = en;
  c.model_config.qwen3_asr.decoder = de; c.model_config.qwen3_asr.tokenizer = tk;
  c.model_config.qwen3_asr.max_total_len = 768; c.model_config.qwen3_asr.max_new_tokens = 256;
  c.model_config.num_threads = 4; c.model_config.provider = "cpu"; c.decoding_method = "greedy_search";
  const SherpaOnnxOfflineRecognizer *r = SherpaOnnxCreateOfflineRecognizer(&c);
  if (!r) { puts("CREATE FAILED"); return 1; }
  const SherpaOnnxWave *w = SherpaOnnxReadWave(wav);
  const SherpaOnnxOfflineStream *s = SherpaOnnxCreateOfflineStream(r);
  if (*hot) SherpaOnnxOfflineStreamSetOption(s, "hotwords", hot);
  SherpaOnnxAcceptWaveformOffline(s, w->sample_rate, w->samples, w->num_samples);
  SherpaOnnxDecodeOfflineStream(r, s);
  const SherpaOnnxOfflineRecognizerResult *res = SherpaOnnxGetOfflineStreamResult(s);
  printf("%.1fs: %s\n", (double)w->num_samples / w->sample_rate, res->text);
  SherpaOnnxDestroyOfflineRecognizerResult(res); SherpaOnnxDestroyOfflineStream(s);
  SherpaOnnxFreeWave(w); SherpaOnnxDestroyOfflineRecognizer(r);
  return 0;
}
