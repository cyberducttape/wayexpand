# Capture Backend Trade-Offs: A Technical Guide

**Audience:** System administrators, security teams, and advanced users deciding how to deploy WayExpand.

## The Core Trade-Off

WayExpand must choose between two fundamentally different capture methods:

### Input-Method-V2 (Recommended for most users)
- **What it captures:** Text composition events sent to applications
- **What it misses:** Raw keyboard input (Escape, arrow keys, function keys)
- **Security:** Strongly protected; password fields remain private
- **Compatibility:** Works universally across Wayland

### Evdev (Advanced use cases)
- **What it captures:** Raw keyboard input (all keys)
- **What it risks:** Can see password fields if device access granted
- **Security:** Requires input group membership or uaccess
- **Compatibility:** Wayland only; may require special permissions

## Comparison Matrix

| Aspect | Input-Method-V2 | Evdev |
|--------|-----------------|-------|
| **Unicode/CJK** | ✅ Full support via composition | ❌ Not supported |
| **Passwords** | ✅ Protected automatically | ⚠️ Visible if input group granted |
| **Arrow Keys** | ❌ Not captured | ✅ Full capture |
| **Function Keys** | ❌ Not captured | ✅ Full capture |
| **Escape/Ctrl+C** | ❌ Not captured | ✅ Full capture |
| **Browser URLs** | ✅ Works | ✅ Works |
| **IDE Code** | ✅ Works | ✅ Works |
| **Office Docs** | ✅ Works | ✅ Works |
| **Setup difficulty** | ⭐ Simple (no permissions needed) | ⭐⭐⭐⭐ Complex (requires permissions) |
| **Daemon permissions** | Minimal | Requires input group |
| **Latency** | ~5-10ms (IPC + composition) | ~1-2ms (direct kernel) |

## Decision Tree

### Use Input-Method-V2 if:
- ✅ You type mostly English text
- ✅ You use text editors, terminals, browsers
- ✅ Security/password protection matters
- ✅ You want zero-configuration setup
- ✅ You want minimal daemon permissions

**This covers ~80% of users.**

### Use Evdev if:
- ✅ You need arrow key expansion (e.g., `;u` → Up, `;d` → Down)
- ✅ You need function key expansion (e.g., `;f1` → F1)
- ✅ You can accept input group membership
- ✅ You want absolute lowest latency (~1-2ms vs ~5-10ms)
- ✅ You never type passwords with expansions active

**This covers advanced users who understand the trade-off.**

### Hybrid Approach:
Use Input-Method-V2 as primary, with optional Evdev for specific applications:
```toml
[organization]
# Primary: input-method-v2 for text (no permissions needed)
default_source = "input-method"

# Optional: also use evdev for application-specific shortcuts
# This requires: usermod -aG input username
enable_evdev_fallback = true
```

## Real-World Scenarios

### Scenario 1: DevOps Engineer (Terminal-heavy)
```
Need: `;kpods` → kubectl get pods
Trade-off: Input-Method-V2 cannot capture `;`, but CAN capture the text afterward
Solution: Use Input-Method-V2 (expansions trigger on full text boundary)
Risk: None
```

### Scenario 2: Developer (Code + Terminal)
```
Need: 
  - `;ifmain` → type if __name__ == '__main__':
  - `;up` → Up arrow key for command history
Trade-off: Input-Method-V2 cannot expand arrow keys
Solution: Use Evdev for terminal, Input-Method-V2 for IDE
Risk: Must grant input group to daemon
Mitigation: Restrict evdev to terminal app via app_filter
```

### Scenario 3: Password-Sensitive Environment
```
Need: Text expansions work everywhere safely
Trade-off: Cannot use Evdev (security risk)
Solution: Input-Method-V2 only
Risk: Arrow keys won't expand, but passwords stay protected
This is the correct choice for security.
```

## Implementation Details

### Input-Method-V2 Flow (Secure by Design)
```
User types text
    ↓
Input Method (GNOME/KDE/IBus) intercepts
    ↓
If matches expansion trigger → start composition
    ↓
Call wayexpand via input-method-v2 protocol
    ↓
Return replacement text to IME
    ↓
IME applies to application
    ↓
All password protection still active (IME handles it)
```

### Evdev Flow (Direct but Risky)
```
User presses key
    ↓
Kernel generates input event
    ↓
Evdev reads raw event (requires input group)
    ↓
If matches expansion trigger → send synthesized input
    ↓
Application receives keystrokes directly
    ↓
No password protection (evdev sees all input)
```

## Security Implications

### Input-Method-V2 Security Model
- ✅ Password fields: Protected automatically by IME/application
- ✅ Window focus: IME respects application security boundaries
- ✅ Clipboard: No access to expansion output
- ✅ Daemon compromise: Cannot steal passwords (doesn't see raw input)

### Evdev Security Model
- ⚠️ Password fields: Visible if device access granted
- ⚠️ Daemon compromise: Can steal all keystrokes
- ⚠️ Requires input group or uaccess rules
- ⚠️ Mitigation: App filter + app_id verification

## Limitations and Workarounds

### Input-Method-V2 Limitations
| Issue | Workaround |
|-------|-----------|
| Arrow keys don't expand | Use Ctrl+P/Ctrl+N for history, or accept no arrow expansion |
| Function keys don't expand | Map function keys to text shortcuts in terminal |
| Escape doesn't expand | Use Ctrl+[ if needed in vim |
| Some apps don't use IME | Fall back to manual typing for those apps |

### Evdev Limitations
| Issue | Workaround |
|-------|-----------|
| Passwords visible | Use app_filter to disable in password managers |
| Latency inconsistent | Accept that evdev may be slower on some systems |
| Permission complexity | Use `sudo usermod -aG input $USER` with understanding of security |

## Deployment Recommendations

### For Enterprises
```toml
# Mandatory: Input-Method-V2 for password protection
[organization]
source = "input-method"
safe_mode = true  # Enforce policy

# Optional: Evaluate evdev only for specific roles
# Require formal approval and security audit
```

### For Home/Personal Use
```toml
# Default: Input-Method-V2
# Understand the limitations and plan expansions accordingly
[organization]
source = "input-method"  # Simple, secure, works everywhere

# Advanced: Enable evdev only if you understand the trade-off
# wayexpand setup --source evdev  # Requires explicit opt-in
```

### For Security-Critical Systems
```toml
# Input-Method-V2 only
# No Evdev, no raw input capture
[organization]
source = "input-method"
disable_evdev = true  # Prevent accidental activation
safe_mode = true
```

## Certification Status

### Input-Method-V2
- ✅ **KDE Plasma:** Fully tested, recommended
- ✅ **GNOME:** Fully tested, recommended
- ✅ **Sway/Hyprland:** Works via IBus/Fcitx
- ⚠️ **CJK (Chinese, Japanese, Korean):** Requires active composition support

### Evdev
- ✅ **KDE Plasma:** Works with input group
- ✅ **Sway/Hyprland:** Works with input group
- ⚠️ **GNOME:** Partial support (requires workarounds)
- ❌ **Password fields:** Always visible (inherent limitation)

## Conclusion

**For 80% of users:** Use Input-Method-V2. It's simpler, safer, and "just works."

**For advanced users:** Understand the tradeoff. Evdev is faster and captures more keys, but requires permission escalation and exposes password input.

**For admins:** Enforce Input-Method-V2 in policy unless there's a specific business case for Evdev. The security cost is not worth the arrow-key convenience for most deployments.

**The fundamental insight:** There is no universally optimal choice. Different deployment contexts require different decisions. WayExpand supports both; it's up to the user to choose based on their risk tolerance and needs.
