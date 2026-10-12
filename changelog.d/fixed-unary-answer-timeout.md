- **A long answer from a slow model no longer times out on `/api/query`.**
  The non-streaming path gave the whole answer 180 seconds, so with a slow
  model (a large MoE with its experts in RAM at ~20 tokens/s, a 24B model
  on a 16 GB card) a long answer ended in a 502 while eullm was still
  writing. It is now read from eullm as a stream and gathered, under the
  streaming path's limits: 30 seconds to connect, at most 180 seconds of
  silence between two pieces, no limit on the total (`num_predict` already
  bounds the length). No new setting.
