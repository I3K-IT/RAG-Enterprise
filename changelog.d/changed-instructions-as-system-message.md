- **The answer instructions go to the model as a system message.** They were
  sent at the top of the one user message, so a chat template that fills an
  absent system message with its own did so on every question: Ministral 3's
  adds its default one, some 525 tokens presenting the model as "Le Chat"
  with today's date, and legal-it-8b, built on it, read that before our
  instructions. The instructions (the defaults or the admin's, from the
  *Answers* tab) are now the system message, and the history, the retrieved
  chunks and the question are the user's turn. A model whose template has
  no system role gets them merged into the user's turn by eullm, as before.
