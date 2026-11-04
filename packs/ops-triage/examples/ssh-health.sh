# Review-only bash commands; environment=staging; SSH target=web-staging
# Expected host: web-01.example.net
# Compare `hostname -f` with the expected host before running any line.
hostname -f
systemctl status --no-pager -- 'api.service'
journalctl -u 'api.service' -n '75' --no-pager
ss -lntp
curl -sS -I --max-time 5 -- http://127.0.0.1:8080/
