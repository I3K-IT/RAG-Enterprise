- **The default instructions keep to the question's language and cite
  without an example.** With English instructions Qwen3.5-9B answered
  Italian questions in English, the language rule being the sixth of seven
  points: it now comes first, and the frame repeats it right before
  `ANSWER:` ("Answer in the language of the QUESTION."), so it holds for
  instructions an admin writes too. The citation rule loses its example,
  which the same model wrote as a citation (`[annual-report-2025.pdf]`).
