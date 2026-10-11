- **eullm 0.7.50, and the update check finds its CUDA build again.** Every
  eullm pin moves from 0.6.90 to 0.7.50; each asset was downloaded and its
  sha256 and size checked against the release's checksums.txt. From 0.7.x
  the CUDA builds are on CUDA 13.1 and named `-cuda-13.1` instead of
  `-cuda-12.8`, so the update check at startup reported "no asset for this
  platform" on every NVIDIA machine; it now looks for the new names. CUDA
  13.1 needs NVIDIA driver 580 or later: on an older driver bootstrap now
  says so and uses the CPU build, instead of installing a binary that
  cannot start.
