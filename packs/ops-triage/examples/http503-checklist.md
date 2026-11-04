HTTP 503 first-response checklist (review each command before running)

Operator-selected environment: staging
Terminal session (operator-confirmed): remote SSH
Expected host: web-01.example.net
Service: api.service
Reverse proxy unit: nginx.service
Local service port: 8080

Before running anything, confirm that this terminal is the selected local or remote SSH session and that `hostname -f` matches the expected host. WayExpand does not inspect terminal contents, detect SSH, or connect to the host. Every command below runs on the machine of the current shell.

1. Confirm the affected URL, start time, scope, and whether the failure is persistent.
2. Check service status, recent logs, and listening sockets:
   ```sh
   hostname -f
   systemctl status --no-pager -- 'api.service'
   journalctl -u 'api.service' -n 100 --no-pager
   ss -lntp
   ```
3. Probe the local service port with the expected virtual host and no response body:
   ```sh
   curl -sS -I --connect-timeout 3 --max-time 5 -H "Host: web-01.example.net" -- 'http://127.0.0.1:8080/'
   ```
4. Check reverse-proxy logs and validate Nginx configuration without reloading:
   ```sh
   journalctl -u 'nginx.service' -n 100 --no-pager
   sudo nginx -t
   ```
5. Compare proxy/upstream health, recent deploys, capacity, and dependency status. Record evidence and escalate before making changes.

These are inserted as editable text and are not executed by WayExpand. The commands inspect state; they do not restart services or change configuration. Redact credentials, tokens, and customer data before sharing output.
