<a name="readme-top"></a>

<!-- PROJECT SHIELDS -->

[![Contributors][contributors-shield]][contributors-url]
[![Forks][forks-shield]][forks-url]
[![Stargazers][stars-shield]][stars-url]
[![Issues][issues-shield]][issues-url]
[![MIT License][license-shield]][license-url]

<!-- PROJECT LOGO -->
<br />
<div align="center">
  <a href="https://github.com/kingwingfly/fav">
    <img src="images/logo.png" alt="Logo" width="80" height="80">
  </a>

<h3 align="center">fav</h3>

  <p align="center">
    Back up your favorite bilibili resources with CLI.
    <br />
    <a href="https://github.com/kingwingfly/fav"><strong>Explore the docs »</strong></a>
    <br />
    <br />
    <a href="https://github.com/kingwingfly/fav">View Demo</a>
    ·
    <a href="https://github.com/kingwingfly/fav/issues">Report Bug</a>
    ·
    <a href="https://github.com/kingwingfly/fav/issues">Request Feature</a>
  </p>
</div>

<!-- TABLE OF CONTENTS -->
<details>
  <summary>Table of Contents</summary>
  <ol>
    <li>
      <a href="#about-the-project">About The Project</a>
      <ul>
        <li><a href="#built-with">Built With</a></li>
      </ul>
    </li>
    <li>
      <a href="#getting-started">Getting Started</a>
      <ul>
        <li><a href="#prerequisites">Prerequisites</a></li>
        <li><a href="#compilation">Compilation</a></li>
      </ul>
    </li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>

<!-- ABOUT THE PROJECT -->

## About The Project

[![Product Name Screen Shot][product-screenshot]](https://github.com/kingwingfly/fav)

Back up your favorite bilibili online resources with CLI.

⚠️: There's a broken change between v0 and v1, details in [CHANGELOG.md](CHANGELOG.md)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

### Built With

- [![Rust][Rust]][Rust-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- GETTING STARTED -->

## Getting Started

You can download the release [here](https://github.com/kingwingfly/fav/releases)

For Arch Linux users, you can `yay -S fav-git` maybe, someone has maken it a package.

Or you can compile by yourself:

1. Install ffmpeg and pkgconf
   You can find method in [this repo's GitHub workflow](.github/workflows/release.yaml).
2. Clone the repo
   ```sh
   git clone https://github.com/kingwingfly/fav.git
   ```
3. Compilation
   ```sh
   cargo build --release
   ```

Or after installing ffmpeg and pkgconf, run `cargo install fav_bili`.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- USAGE EXAMPLES -->

## Usage

Need `libav*` able to be dynamic linked (`pkgconf --list-all | grep libav`).

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
fav auth login # you can also login with `fav usecookies`
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
# or like all medias faved
fav ls v | sed '1d;$d' | awk '{print $2;}' | xargs fav like
# check cookies usability
fav auth check -a
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

You can also achieve the goal with `systemd timer` by yourself, but it's a little hard to learn.

_For more examples, please refer to the [Documentation](https://github.com/kingwingfly/fav)_

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- DEVELOP -->

## Develop

`sea-orm-cli` is used to handle database ops.

```sh
cargo binstall sea-orm-cli # or `cargo install sea-orm-cli`
# generate ORM code
./sea-orm.sh
```

<!-- CONTRIBUTING -->

## Contributing

Contributions are what make the open source community such an amazing place to learn, inspire, and create. Any contributions you make are **greatly appreciated**. Moreover, it is recommended to open an issue before coding to avoid repeated and useless work.

If you have a suggestion that would make this better, please fork the repo and create a pull request. You can also simply open an issue with the tag "enhancement".
Don't forget to give the project a star! Thanks again!

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- LICENSE -->

## License

Distributed under the MIT License. See `LICENSE.txt` for more information.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- CONTACT -->

## Contact

Louis - 836250617@qq.com

Project Link: [https://github.com/kingwingfly/fav](https://github.com/kingwingfly/fav)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- ACKNOWLEDGMENTS -->

## Acknowledgments

- [bilibili-API-collect](https://github.com/SocialSisterYi/bilibili-API-collect)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- MARKDOWN LINKS & IMAGES -->
<!-- https://www.markdownguide.org/basic-syntax/#reference-style-links -->

[contributors-shield]: https://img.shields.io/github/contributors/kingwingfly/fav.svg?style=for-the-badge
[contributors-url]: https://github.com/kingwingfly/fav/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/kingwingfly/fav.svg?style=for-the-badge
[forks-url]: https://github.com/kingwingfly/fav/network/members
[stars-shield]: https://img.shields.io/github/stars/kingwingfly/fav.svg?style=for-the-badge
[stars-url]: https://github.com/kingwingfly/fav/stargazers
[issues-shield]: https://img.shields.io/github/issues/kingwingfly/fav.svg?style=for-the-badge
[issues-url]: https://github.com/kingwingfly/fav/issues
[license-shield]: https://img.shields.io/github/license/kingwingfly/fav.svg?style=for-the-badge
[license-url]: https://github.com/kingwingfly/fav/blob/master/LICENSE.txt
[product-screenshot]: images/screenshot.png
[Rust]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=Rust&logoColor=orange
[Rust-url]: https://www.rust-lang.org
