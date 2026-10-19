# 3. Decide request

**Inputs:** given by [4. One program connection](4-program-connection.md):
- `settings`: the checked settings file, as given to [4. One program connection](4-program-connection.md)
- `request`: parsed request head, with header names in lowercase

**Output:** `Release { keys, host }  |  Pass { host }  |  Refuse(Refusal)`
- `Refusal { reason, keys, host }`

**Calls:** none.

**Called by:** [4. One program connection](4-program-connection.md), at step 2.

## Pseudo code

This protocol has no diagram of its own: its reference is step 2 of the [sequence diagram](4-program-connection.puml) of [4. One program connection](4-program-connection.md). Each case below is a row of the cases there, in the same order.

```text
// A key counts only when its placeholder is in its own bound header; anywhere else, it's plain text.
function decide(request, settings):
    url_host = normalize(request.target.host, request.target.port)   // lowercase, with :80 added when the port is missing
    host_headers = [normalize_host_header(value) for value in request.headers.values_of("Host")]

    keys_found = [key for key in settings.keys
                  if any(key.placeholder in value for value in request.headers.values_of(key.header))]

    // Case 1: host_mismatch
    if host_headers != [url_host]:
        return Refuse(Refusal(host_mismatch, keys_found, url_host))

    // Case 2: malformed_request (a bound header appears more than once)
    if any(request.headers.count(key.header) > 1 for key in keys_found):
        return Refuse(Refusal(malformed_request, keys_found, url_host))

    // Case 3: host_not_allowed (a key not allowed for this host)
    if any(key.host != url_host for key in keys_found):
        return Refuse(Refusal(host_not_allowed, keys_found, url_host))

    // Case 4: host_not_allowed (no key, host not listed)
    if keys_found is empty and url_host not in settings.hosts:
        return Refuse(Refusal(host_not_allowed, keys_found, url_host))

    // Case 5: pass
    if keys_found is empty:
        return Pass(url_host)

    // Case 6: release
    return Release(keys_found, url_host)
```
