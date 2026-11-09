
trap '' HUP INT TERM
while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
app=$2 new=$3 old=$4
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then
    rm -rf "$old"
  else
    rm -rf "$app"
    mv "$old" "$app"
    echo "$6" > "$5"
  fi
else
  echo "$6" > "$5"
fi
[ -n "$7" ] || exit 0
if [ -n "$8" ]; then open -n --env "NODUS_USER_DATA=$8" "$app"; else open -n "$app"; fi
