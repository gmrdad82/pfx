Hide
Type `git init -q w && pfx new w > /dev/null 2>&1; pfx bench w/content/first.pitofx --seconds 1 > /dev/null 2>&1; rm -rf w; echo warm""-done`
Enter
Wait+Screen@300s /warm-done/
Type "clear"
Enter
Sleep 300ms
Show
Sleep 500ms
Type@40ms "pfx --version"
Enter
Sleep 800ms
Type@40ms "git init -q demo && pfx new demo"
Enter
Sleep 1500ms
Type@40ms "ls demo demo/content"
Enter
Sleep 1500ms
Type@40ms "pfx bench demo/content/first.pitofx --seconds 3 2>/dev/null | jq '{frames, dropped, gpu_ms}'"
Enter
Wait+Screen@300s /"dropped": [0-9]+/
Sleep 4s
