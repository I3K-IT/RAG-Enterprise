- **A streamed answer is cleaned like any other.** Since 0.2.0 a reasoning
  block or a chat template marker that a model emits despite `think: false`
  was removed from unary answers only: the streamed answer the interface
  shows, and the reply stored with the conversation, kept them. The stream
  is now cleaned piece by piece, holding back whatever could still be the
  start of a marker split across pieces.
