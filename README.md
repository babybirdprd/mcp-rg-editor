# Tauri 2.0 + Next.js 15 App Router Template

![Tauri window screenshot](public/tauri-nextjs-template-2_screenshot.png)

This is a [Tauri](https://v2.tauri.app/) project template using [Next.js](https://nextjs.org/),
bootstrapped by combining [`create-next-app`](https://github.com/vercel/next.js/tree/canary/packages/create-next-app)
and [`create tauri-app`](https://v2.tauri.app/start/create-project/).

This template uses [`pnpm`](https://pnpm.io/) as the Node.js dependency
manager, and uses the [App Router](https://nextjs.org/docs/app) model for Next.js.

## Template Features

- TypeScript frontend using [Next.js 15](https://nextjs.org/) React framework
- [TailwindCSS 4](https://tailwindcss.com/) as a utility-first atomic CSS framework
  - The example page in this template app has been updated to use only TailwindCSS
  - While not included by default, consider using
    [React Aria components](https://react-spectrum.adobe.com/react-aria/index.html)
    and/or [HeadlessUI components](https://headlessui.com/) for completely unstyled and
    fully accessible UI components, which integrate nicely with TailwindCSS
- Opinionated formatting and linting already setup and enabled
  - [Biome](https://biomejs.dev/) for a combination of fast formatting, linting, and
    import sorting of TypeScript code, and [ESLint](https://eslint.org/) for any missing
    Next.js linter rules not covered by Biome
  - [clippy](https://github.com/rust-lang/rust-clippy) and
    [rustfmt](https://github.com/rust-lang/rustfmt) for Rust code
- GitHub Actions to check code formatting and linting for both TypeScript and Rust

## Getting Started

### Running development server and use Tauri window

After cloning for the first time, change your app identifier inside
`src-tauri/tauri.conf.json` to your own:

```jsonc
{
  // ...
  // The default "com.tauri.dev" will prevent you from building in release mode
  "identifier": "com.my-application-name.app",
  // ...
}
```

To develop and run the frontend in a Tauri window:

```shell
pnpm tauri dev
```

This will load the Next.js frontend directly in a Tauri webview window, in addition to
starting a development server on `localhost:3000`.
Press <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> in a Chromium based WebView (e.g. on
Windows) to open the web developer console from the Tauri window.

### Building for release

To export the Next.js frontend via SSG and build the Tauri application for release:

```shell
pnpm tauri build
```

### Source structure

Next.js frontend source files are located in `src/` and Tauri Rust application source
files are located in `src-tauri/`. Please consult the Next.js and Tauri documentation
respectively for questions pertaining to either technology.

## Caveats

### Static Site Generation / Pre-rendering

Next.js is a great React frontend framework which supports server-side rendering (SSR)
as well as static site generation (SSG or pre-rendering). For the purposes of creating a
Tauri frontend, only SSG can be used since SSR requires an active Node.js server.

Please read into the Next.js documentation for [Static Exports](https://nextjs.org/docs/app/building-your-application/deploying/static-exports)
for an explanation of supported / unsupported features and caveats.

### `next/image`

The [`next/image` component](https://nextjs.org/docs/basic-features/image-optimization)
is an enhancement over the regular `<img>` HTML element with server-side optimizations
to dynamically scale the image quality. This is only supported when deploying the
frontend onto Vercel directly, and must be disabled to properly export the frontend
statically. As such, the
[`unoptimized` property](https://nextjs.org/docs/api-reference/next/image#unoptimized)
is set to true for the `next/image` component in the `next.config.js` configuration.
This will allow the image to be served as-is, without changes to its quality, size,
or format.

### ReferenceError: window/navigator is not defined

If you are using Tauri's `invoke` function or any OS related Tauri function from within
JavaScript, you may encounter this error when importing the function in a global,
non-browser context. This is due to the nature of Next.js' dev server effectively
running a Node.js server for SSR and hot module replacement (HMR), and Node.js does not
have a notion of `window` or `navigator`.

The solution is to ensure that the Tauri functions are imported as late as possible
from within a client-side React component, or via [lazy loading](https://nextjs.org/docs/app/building-your-application/optimizing/lazy-loading).

## MCP Transport Mode and File Root Configuration

This app uses an environment variable `MCP_TRANSPORT` to control how the backend server communicates. You can set this variable at runtime to choose between different transport modes:

- `stdio` — Use standard input/output (default if only `mcp-stdio-server` feature is enabled)
- `sse` — Use server-sent events (default if only `mcp-sse-server` feature is enabled, or if both features are enabled)
- `disabled` — Disable the MCP server

**How to set at runtime (PowerShell example):**

```powershell
$env:MCP_TRANSPORT = "sse"
pnpm tauri dev --features "mcp-sse-server mcp-stdio-server"
```
Or for stdio:
```powershell
$env:MCP_TRANSPORT = "stdio"
pnpm tauri dev --features "mcp-sse-server mcp-stdio-server"
```

You can also set this in a `.env` file in the `src-tauri` directory:
```
MCP_TRANSPORT=sse
```

### Default URLs and Ports for MCP Transport Modes

- **stdio**: The backend communicates over standard input/output (stdio) and does not expose a network port or URL. This mode is only accessible internally by the Tauri app and is not reachable from external clients.
- **sse**: The backend starts a local HTTP server for Server-Sent Events (SSE) on `http://127.0.0.1:14338` by default. The frontend connects to this URL to receive events and communicate with the backend. You can override the port by setting the `MCP_SSE_PORT` environment variable (e.g., `MCP_SSE_PORT=14338`).

The file root for all file operations is set by the `FILES_ROOT` environment variable (or in your `.env` file). This must be an absolute path or a path like `~/mcp_files`. If `FILES_ROOT` is not set, the app will not start and will show an error. Example for PowerShell:

```powershell
$env:FILES_ROOT = "C:/Users/YourName/mcp_files"
```
Or in your `.env` file in `src-tauri`:
```
FILES_ROOT=C:/Users/YourName/mcp_files
```

If you want to allow access to additional directories, set the `ALLOWED_DIRECTORIES` environment variable (comma-separated list of absolute paths). By default, only `FILES_ROOT` is allowed.

---

**Planned improvement:**
> In the future, the app will provide a user-friendly wizard or installer to let users pick the transport mode and file root, so they never have to deal with environment variables or config files directly.

## Learn More

To learn more about Next.js, take a look at the following resources:

- [Next.js Documentation](https://nextjs.org/docs) - learn about Next.js features and
  API.
- [Learn Next.js](https://nextjs.org/learn) - an interactive Next.js tutorial.

And to learn more about Tauri, take a look at the following resources:

- [Tauri Documentation - Guides](https://v2.tauri.app/start/) - learn about the Tauri
  toolkit.
