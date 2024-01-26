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
  <a href="https://github.com/kingwingfly/repo_name">
    <img src="images/logo.png" alt="Logo" width="80" height="80">
  </a>

<h3 align="center">backup</h3>

  <p align="center">
    Back up your favorite online resources just like you would with git.
    <br />
    <a href="https://github.com/kingwingfly/repo_name"><strong>Explore the docs »</strong></a>
    <br />
    <br />
    <a href="https://github.com/kingwingfly/repo_name">View Demo</a>
    ·
    <a href="https://github.com/kingwingfly/repo_name/issues">Report Bug</a>
    ·
    <a href="https://github.com/kingwingfly/repo_name/issues">Request Feature</a>
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
        <li><a href="#installation">Installation</a></li>
      </ul>
    </li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#roadmap">Roadmap</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>



<!-- ABOUT THE PROJECT -->
## About The Project

[![Product Name Screen Shot][product-screenshot]](https://example.com)

Back up your favorite online resources just like you would with git.

<p align="right">(<a href="#readme-top">back to top</a>)</p>



### Built With

* [![Rust][Rust]][Rust-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- GETTING STARTED -->
## Getting Started

### Prerequisites

* Install Rust
  ```sh
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

### Compilation

1. Clone the repo
   ```sh
   git clone https://github.com/kingwingfly/backup.git
   ```
2. Compilation
   ```sh
   cargo build
   ```

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- USAGE EXAMPLES -->
## Usage

```sh
$ backup -h
Back up your favorite online resources just like you would with git.

Usage: backup <COMMAND>

Commands:
  init    Initialize the folder for backup
  login   Login your account
  fetch   Fetch from remote
  status  Show status of local
  track   Track a remote source
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### Example
```sh
# init for bilibili
$ backup init bili
# scan code to login
$ backup login qr-code
# fetch favorite list
$ backup fetch
# show status
$ backup status -l
# track list (just like `git add`, by the way, commit is not needed)
$ backup track <list_id/bvid>
# fetch tracked
$ backup fetch
# pull videos (support dry-run)
$ backup pull
# push videos (support dry-run)
$ backup push
# ignore list or video
$ backup ignore <list_id/bvid>
# untrack list or video (just like `git rm`)
$ backup untrack <list_id/bvid>
```


_For more examples, please refer to the [Documentation](https://example.com)_

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- ROADMAP -->
## Roadmap

- [x] Init
- [x] Login
- [x] Fetch
- [x] Status
- [x] Track
- [x] Untrack
- [ ] Pull
- [ ] Push
- [ ] Ignore
- [ ] Sync

See the [open issues](https://github.com/kingwingfly/backup/issues) for a full list of proposed features (and known issues).

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- CONTRIBUTING -->
## Contributing

Contributions are what make the open source community such an amazing place to learn, inspire, and create. Any contributions you make are **greatly appreciated**. Moreover, it is recommended to open an issue before coding to avoid repeated and useless work.

If you have a suggestion that would make this better, please fork the repo and create a pull request. You can also simply open an issue with the tag "enhancement".
Don't forget to give the project a star! Thanks again!

1. Fork the Project
2. Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3. Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the Branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- LICENSE -->
## License

Distributed under the MIT License. See `LICENSE.txt` for more information.

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- CONTACT -->
## Contact

Louis - 836250617@qq.com

Project Link: [https://github.com/kingwingfly/backup](https://github.com/kingwingfly/backup)

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- ACKNOWLEDGMENTS -->
## Acknowledgments

* [bilibili-API-collect](https://github.com/SocialSisterYi/bilibili-API-collect)

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- MARKDOWN LINKS & IMAGES -->
<!-- https://www.markdownguide.org/basic-syntax/#reference-style-links -->
[contributors-shield]: https://img.shields.io/github/contributors/kingwingfly/backup.svg?style=for-the-badge
[contributors-url]: https://github.com/kingwingfly/backup/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/kingwingfly/backup.svg?style=for-the-badge
[forks-url]: https://github.com/kingwingfly/backup/network/members
[stars-shield]: https://img.shields.io/github/stars/kingwingfly/backup.svg?style=for-the-badge
[stars-url]: https://github.com/kingwingfly/backup/stargazers
[issues-shield]: https://img.shields.io/github/issues/kingwingfly/backup.svg?style=for-the-badge
[issues-url]: https://github.com/kingwingfly/backup/issues
[license-shield]: https://img.shields.io/github/license/kingwingfly/backup.svg?style=for-the-badge
[license-url]: https://github.com/kingwingfly/backup/blob/master/LICENSE.txt
[product-screenshot]: images/screenshot.png
[Rust]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=Rust&logoColor=orange
[Rust-url]: https://www.rust-lang.org
