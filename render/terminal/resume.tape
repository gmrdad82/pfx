Hide
Type `pfx new street.pitofx --from street > /dev/null && pfx clip street.pitofx --camera chase --timeline --size 480x270 --trace --spp 64 --range 0..3 --out w > /dev/null 2>&1; rm -rf w street.pitofx; echo warm""-done`
Enter
Wait+Screen@300s /warm-done/
Type "clear"
Enter
Sleep 300ms
Show
Sleep 500ms
Type@40ms "pfx new street.pitofx --from street"
Enter
Sleep 600ms
Type@40ms "f='street.pitofx --camera chase --timeline --size 480x270 --trace --spp 64'"
Enter
Sleep 400ms
Type@40ms "pfx clip $f --range 0..3 --out whole 2> /dev/null"
Enter
Wait+Screen@300s /frame-0003\.png/
Sleep 800ms
Type@40ms "pfx job start cut --frames 0..3 -- clip $f --out cut > /dev/null"
Enter
Sleep 400ms
Type@10ms `until pfx job status cut --json | jq -e '.frames.done == 2 and .samples.done > 0' > /dev/null; do sleep 0.1; done; kill -9 -- $(pfx job status cut --json | jq -r '"-\(.supervisor.pid) -\(.child.pid)"'); sleep 1; pfx job list; ls cut`
Enter
Wait+Screen@120s /supervisor is gone/
Sleep 2500ms
Type@40ms "pfx job resume cut > /dev/null"
Enter
Type@40ms `until pfx job status cut --json | jq -e '.state == "done"' > /dev/null; do sleep 0.2; done; pfx job list`
Enter
Wait+Screen@300s /cut +done/
Sleep 800ms
Type@40ms `for p in whole/frame-*.png; do cmp $p cut/${p#whole/} && echo "same bytes: ${p#whole/}"; done`
Enter
Sleep 4s
