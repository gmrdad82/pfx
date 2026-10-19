Sleep 500ms
Type@40ms "pfx --version"
Enter
Sleep 800ms
Type@40ms "git init -q demo && pfx new demo"
Enter
Sleep 1500ms
Type@40ms "cd demo && ls . content"
Enter
Sleep 1500ms
Type@40ms "pfx bench content/first.pitofx --seconds 3 2>/dev/null | jq '{frames, dropped, gpu_ms}'"
Enter
Wait+Screen@90s /"p99": [0-9.]+/
Sleep 3s
