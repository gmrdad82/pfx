Sleep 500ms
Type@40ms "git init -q scenes && cd scenes"
Enter
Sleep 600ms
Type@40ms "pfx new street.pitofx --from street && git add street.pitofx"
Enter
Sleep 900ms
Type@40ms "pfx set street.pitofx sun.hour=7.5"
Enter
Sleep 1200ms
Type@40ms "git --no-pager diff"
Enter
Sleep 1500ms
Type@40ms "pfx git setup"
Enter
Sleep 1500ms
Type@40ms "git --no-pager diff"
Enter
Wait+Screen@30s /\+.*hour/
Sleep 4s
