Sleep 500ms
Type@40ms "pfx new street.pitofx --from street && cp street.pitofx before.pitofx"
Enter
Sleep 800ms
Type@40ms "pfx set street.pitofx sun.hour=7.5 timeline.seconds=6"
Enter
Sleep 1500ms
Type@40ms "pfx set street.pitofx sun.hour=31"
Enter
Sleep 1800ms
Type@40ms "diff <(pfx scene print before.pitofx) <(pfx scene print street.pitofx)"
Enter
Sleep 2000ms
Type@40ms "pfx build street.pitofx && du -h out/*"
Enter
Wait+Screen@60s /street\.glb/
Sleep 4s
