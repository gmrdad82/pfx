Sleep 500ms
Type@40ms "pfx --version"
Enter
Sleep 800ms
Type@40ms "git init -q demo && pfx new demo --path demo"
Enter
Sleep 1s
Type@40ms "cd demo && ls . content"
Enter
Sleep 2s
Type@40ms "clear"
Enter
Type@40ms "pfx bench content/first.scene.toml --seconds 3 --size 1920x1080 2>/dev/null | jq -c '{frames,gpu_ms}'"
Enter
Wait+Screen@90s /"frames":[0-9]+/
Sleep 2500ms
