This is a package for controlling the OBSBot Tiny 2 in Linux.  It includes a gui, OSC server, CLI tool, and V4L2 controls.  The communication is self-contained; it does not rely on any OBSBOT code.

- `obsbot-gui` is a gui dashboard for control.
- `obsbot-osc-server`  is an Open Sound Control server.
- `obsbot-v4l2-mapper` is a CLI that allows installation of V4L2 controls, making control of the camera possible in, eg, OBS Studio.

Installation should be possible using standard Cargo commands, eg,

```bash
cargo install --locked --git https://github.com/cgevans/tiny2
```

The core of the code was originally based on [samliddicott's meet4k package](https://github.com/samliddicott/meet4k), though it has been substantially changed.

I'm open to contributions to the package, and I'm not opposed to AI use: I have used it substantially for the more recent code here.  I may not have time to review contributions, but will try to; I also only have one Tiny 2, and sometimes am away from it for extended periods of time.  But if you submit a PR please show enough decency to have at least the explanation of it be something you wrote, and disclose what communication is yours and what is AI; if you submit wide-ranging AI-written PRs without a single word of human explanation I'll just block you.  If that's how you'd prefer to interact, please just maintain your own fork, while respecting the license.