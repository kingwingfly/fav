A CLI tool to download your favorite Bilibili medias from up and favorite collections.

Need `ffmpeg` usable, and able to be directly called in cli.

```sh
Back up your favorite bilibili online resources with CLI.

Usage: fav [OPTIONS] [COMMAND]

Commands:
  auth        Auth account
  list        List accounts/sets/ups/medias [alias: ls, l]
  activate    Activate obj [alias: active, a]
  deactivate  Deactivate obj [alias: d]
  fetch       Fetch metadata of following ups, fav sets, medias, ups [alias: f]
  pull        Pull fetched medias [alias: p]
  like        Like medias
  completion  Generate completion script
  help        Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose  Show debug messages
  -h, --help     Print help
  -V, --version  Print version
```

### Steps

1. Login first
2. Fetch the favorite sets(lists)
3. Activate the list or up you want. You can see them through `fav ls`
4. Fetch active resources
5. Pull the resources

### Example

```sh
# auto completion is supported; e.g. fish
fav completion fish > ~/.config/fish/completions/fav.fish
# For Windows users
echo "fav completion powershell | Out-String | Invoke-Expression" >> $PROFILE
# scan code to login
fav auth login
# you can also login with `fav usecookies`
# fetch following ups and fav sets
fav fetch
# show sets
fav ls set
# activate set or up
fav activate
# pull videos
fav fetch
fav pull
# deactivate set or up
fav deactivate
# after fetching, you can find your favorite upper
# limbo/sqlite3 .fav/fav.db
SELECT u.up_id, u.name, COUNT(u.up_id) count FROM up u LEFT JOIN media_up mu ON u.up_id=mu.up_id JOIN media m ON mu.id=m.id GROUP BY u.up_id, u.name ORDER BY count;
# you can also like medias, should usecookies when login
fav like
```

Service example:
```ini
# /etc/systemd/system/fav.service
[Unit]
Description=Fav Service
After=network-online.target

[Service]
Type=oneshot
User=your_user
WorkingDirectory=/path/to/fav_set
ExecStart=/bin/sh -c "/usr/local/bin/fav fetch && /usr/local/bin/fav pull"

# /etc/systemd/system/fav.timer
[Unit]
Description=Run fav service every 3 hours

[Timer]
OnCalendar=*-*-* 0/3:00:00
# or OnUnitActiveSec=3h
AccuracySec=1m
Persistent=true

[Install]
WantedBy=timers.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable fav.timer
sudo systemctl start fav.timer
```
