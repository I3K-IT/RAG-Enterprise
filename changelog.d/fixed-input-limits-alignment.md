- **Form inputs now match the server-side limits.** The question box
  had no cap while over-4000-character questions get a 400, and the
  new-password fields stopped at a 6 minimum while over-128-character
  passwords get a 400. Both now stop at the boundary (`maxLength`),
  with the password hint stating the real 6-128 range.
