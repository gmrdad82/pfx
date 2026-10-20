Hide
Type `git init -q w && pfx new w > /dev/null 2>&1; pfx shot w/content/first.pitofx --seconds 1 --fps 30 --size 640x360 --draft --out tmp/w > /dev/null 2>&1; rm -rf w tmp/w; echo warm""-done`
Enter
Wait+Screen@300s /warm-done/
Type "clear"
Enter
Sleep 300ms
Show
Sleep 500ms
Type@40ms "git init -q demo && pfx new demo > /dev/null"
Enter
Sleep 1s
Type@40ms "pfx shot demo/content/first.pitofx --seconds 2 --fps 30 --size 640x360 --draft --out tmp/shot > /dev/null"
Enter
Wait+Screen@120s /shot: peak memory/
Sleep 800ms
Type@25ms `cat > short.film.toml <<'EOF'`
Enter
Type@15ms `scene = [`
Enter
Type@15ms `  { name = "open", kind = "card", seconds = 1.0, gradient = ["#1d2733", "#0b0f14"] },`
Enter
Type@15ms `  { name = "shot", kind = "frames", dir = "tmp/shot", seconds = 2.0 },`
Enter
Type@15ms `  { name = "close", kind = "card", seconds = 1.0, colour = "#b9e05a" },`
Enter
Type@15ms `]`
Enter
Type@15ms `[film]`
Enter
Type@15ms `size = [640, 360]`
Enter
Type@15ms `layout = [640, 360]`
Enter
Type@15ms `fps = 30`
Enter
Type@15ms `EOF`
Enter
Sleep 600ms
Type@40ms "pfx film short.film.toml --out tmp/film > /dev/null"
Enter
Wait+Screen@120s /peak memory/
Sleep 800ms
Type@40ms "ls tmp/film; ls tmp/film/frames | wc -l"
Enter
Sleep 4s
