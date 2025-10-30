## Using fzf with HandControl Pairing Approval

The `list-pending` command outputs tab-separated values that are perfect for piping to fzf.

### Output Format

The `list-pending` command outputs the following columns (tab-separated):

1. `request_id` - Unique identifier for the pairing request
2. `device_name` - Name of the device requesting pairing
3. `device_model` - Model of the device (or "Unknown")
4. `verification_code` - PIN code for verification
5. `expires_in` - Time remaining in seconds

### Quick One-Liner

```bash
handcontrol approve $(handcontrol list-pending | fzf --delimiter='\t' --with-nth=2,3,4,5 | cut -f1)
```

This will:
1. List all pending requests
2. Show device name, model, PIN, and expiry in fzf
3. Extract the request_id from the selected line
4. Approve the selected request

### Using the Example Script

For a more user-friendly experience with a preview window:

```bash
./examples/approve-with-fzf.sh
```

### Manual Inspection

To manually view pending requests:

```bash
handcontrol list-pending
```

Example output:
```
a1b2c3d4-e5f6-7890-abcd-ef1234567890    Pixel 8 Pro    Google Pixel 8 Pro    123-456    285s
b2c3d4e5-f6a7-8901-bcde-f12345678901    Galaxy S24     Samsung Galaxy S24    789-012    142s
```

### Using with column for Pretty Display

```bash
handcontrol list-pending | column -t -s $'\t' -N "Request ID,Device,Model,PIN,Expires"
```
