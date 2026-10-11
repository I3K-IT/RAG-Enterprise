- **Answers use the chat format of the model eullm has loaded, so models
  other than Qwen work.** The prompt used to be wrapped in Qwen's ChatML by
  hand and sent raw: with a Ministral 8B that meant stray `</answer>` tags
  in 74 answers out of 472 and a file cited in only 99. It now goes to
  eullm's `/api/chat`, which applies the loaded model's own template, with
  `think: false` to switch reasoning off; the `/no_think` line at the top of
  the prompt and the empty reasoning block appended to it are gone, both
  were Qwen conventions. A reasoning block or template marker that a model
  emits anyway is still removed from the answer.
