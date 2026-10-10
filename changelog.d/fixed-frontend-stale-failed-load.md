- **A conversation load that fails after you have moved on no longer empties
  the one you are reading.** The guard added for late replies now covers
  late failures too: switching threads while a slow load is in flight, then
  having that load fail, used to clear the messages of the thread on screen.
