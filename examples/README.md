# HandControl Configuration Examples

This directory contains example configuration files to help you get started with HandControl quickly.

## Files

- **basic_config.toml** - Simple configuration with common commands (Linux)
- **advanced_config.toml** - Comprehensive example showing all parameter types (Linux)
- **windows_config.toml** - Windows-specific commands and tools
- **macos_config.toml** - macOS-specific commands using AppleScript and system tools

## Installation

### Linux

Copy the desired configuration to your config directory:

```bash
mkdir -p ~/.config/handcontrol
cp examples/basic_config.toml ~/.config/handcontrol/config.toml
```

### Windows

```powershell
mkdir $env:APPDATA\handcontrol
copy examples\windows_config.toml $env:APPDATA\handcontrol\config.toml
```

### macOS

```bash
mkdir -p ~/Library/Application\ Support/handcontrol
cp examples/macos_config.toml ~/Library/Application\ Support/handcontrol/config.toml
```

## Configuration Structure

All configuration files follow this structure:

```toml
[server]
bind_address = "0.0.0.0"
port = 50051
mdns_instance_name = "My Computer"

[security]
enrollment_token_ttl = 300  # seconds

[security.enrollment]
qr_code_enabled = true
approval_enabled = true
approval_notification = true
approval_timeout_seconds = 60

[commands]
timeout_seconds = 30

[[command]]
id = "command-id"
name = "Display Name"
description = "What this command does"
shell = "command to execute"
confirm = false  # Optional: require confirmation

[[command.parameters]]
name = "param_name"
display_name = "Parameter Label"
type = "slider"  # or "text", "toggle", "selection"
# Type-specific fields (min/max for slider, options for selection, etc.)
```

## Parameter Types

HandControl supports four parameter types:

### 1. Slider (Numeric)

```toml
[[command.parameters]]
name = "volume"
display_name = "Volume Level"
type = "slider"
min = 0
max = 100
default = 50
```

### 2. Text

```toml
[[command.parameters]]
name = "message"
display_name = "Message"
type = "text"
regex = "^[a-zA-Z0-9 ]+$"  # Optional validation
default = "Hello"
```

### 3. Toggle (Boolean)

```toml
[[command.parameters]]
name = "enabled"
display_name = "Enable Feature"
type = "toggle"
default = true
```

### 4. Selection (Dropdown)

```toml
[[command.parameters]]
name = "output"
display_name = "Output Device"
type = "selection"
options = ["Speakers", "Headphones", "HDMI"]
default = "Speakers"
```

## Platform-Specific Notes

### Linux

Most commands in `basic_config.toml` and `advanced_config.toml` require common Linux tools:

- **playerctl** - Media controls (install: `sudo apt install playerctl`)
- **pactl** (PulseAudio) - Volume control (usually pre-installed)
- **scrot** - Screenshots (install: `sudo apt install scrot`)
- **brightnessctl** - Brightness control (install: `sudo apt install brightnessctl`)
- **wmctrl** - Window management (install: `sudo apt install wmctrl`)
- **xclip** - Clipboard operations (install: `sudo apt install xclip`)

### Windows

The Windows configuration uses:

- **NirCmd** - Powerful command-line utility for Windows
  - Download from: https://www.nirsoft.net/utils/nircmd.html
  - Extract and add to your PATH

- **PowerShell** - Built-in for clipboard and some system operations

### macOS

The macOS configuration primarily uses:

- **osascript** (AppleScript) - Built-in scripting
- **pmset** - Power management (built-in)
- **screencapture** - Screenshots (built-in)
- **pbcopy/pbpaste** - Clipboard operations (built-in)

## Security Considerations

1. **Shell Command Safety**: All commands are executed through the shell. Only configure commands you trust.

2. **Parameter Validation**: Use regex validation for text parameters to prevent command injection:
   ```toml
   regex = "^[a-zA-Z0-9_./\\- ]+$"  # Allow only safe characters
   ```

3. **Confirmation**: Add `confirm = true` for destructive operations:
   ```toml
   [[command]]
   id = "shutdown"
   shell = "systemctl poweroff"
   confirm = true
   ```

4. **Client Authentication**: Only enrolled devices with valid certificates can execute commands.

## Customization Tips

1. **Test Commands First**: Run commands manually in your terminal before adding them to the config.

2. **Use Environment Variables**: Commands can access environment variables:
   ```toml
   shell = "echo $USER > $HOME/user.txt"
   ```

3. **Output Redirection**: Capture output to files for later review:
   ```toml
   shell = "system-info > /tmp/sysinfo.txt 2>&1"
   ```

4. **Multiple Parameters**: Combine parameters in commands:
   ```toml
   [[command]]
   id = "custom-notification"
   shell = "notify-send '{title}' '{message}'"

   [[command.parameters]]
   name = "title"
   type = "text"

   [[command.parameters]]
   name = "message"
   type = "text"
   ```

5. **Chaining Commands**: Use shell operators to chain commands:
   ```toml
   shell = "cd ~/Documents && ls -la > file-list.txt"
   ```

## Troubleshooting

### Commands Not Working

1. Check that required tools are installed and in PATH
2. Test the command directly in your terminal
3. Check server logs for error messages
4. Verify parameter substitution syntax: `{parameter_name}`

### Permission Errors

Some commands may require elevated privileges:

```bash
# Run server with sudo (not recommended for daily use)
sudo handcontrol serve

# Or configure specific commands with sudo (better approach)
shell = "sudo systemctl suspend"
```

For sudo without password, configure sudoers:
```bash
sudo visudo
# Add: username ALL=(ALL) NOPASSWD: /usr/bin/systemctl suspend
```

### Timeout Issues

Increase timeout for long-running commands:

```toml
[commands]
timeout_seconds = 120  # 2 minutes

# Or per-command:
[[command]]
timeout_seconds = 300  # 5 minutes for this specific command
```

## Contributing

Have a useful command configuration? Consider sharing it by:

1. Adding it to one of the example files
2. Creating a pull request
3. Including a description and any required dependencies

## Resources

- Main Documentation: `../docs/PRD.md`
- Security Guide: `../docs/SECURITY.md`
- Project Structure: `../docs/PROJECT_STRUCTURE.md`
