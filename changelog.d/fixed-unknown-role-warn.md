- **An unknown role in the database now logs instead of silently
  demoting.** `UserRow::role()` fell back to `User` on any unparsable
  value, and the role is re-read from the row on login and on every
  request — so a typo or a corrupt row could demote anyone, admin
  included, with nothing in the log saying why. Still fail-closed
  toward the least privilege, now with a warning naming the user and
  the offending value.
