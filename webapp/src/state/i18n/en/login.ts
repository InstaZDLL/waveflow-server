// Sign-in and first-run setup.
export const login = {
  "login.heroEyebrow": "Your music, on your terms",
  "login.heroTitle": "After dark, the library comes alive.",
  "login.heroDetail": "Private streaming, one catalogue, every screen.",
  "login.server": "WaveFlow server",
  "login.createAdmin": "Create the administrator",
  "login.welcome": "Welcome back",
  "login.setupDetail":
    "This is a new server. Choose the first administrator account.",
  "login.setupToken": "Setup token",
  "login.setupTokenHint":
    "The server wrote it to the file setup-token in its data directory, and its log names the path. If you set WAVEFLOW_SETUP_TOKEN, enter that value.",
  "login.setupTokenError":
    "This setup token is not the right one. Copy it again from the setup-token file.",
  "login.username": "Username",
  "login.password": "Password",
  "login.setupError":
    "Setup failed. Use a valid username and at least 12 password characters.",
  "login.error": "Wrong username or password.",
  "login.wait": "Please wait…",
  "login.create": "Create and sign in",
  "login.signIn": "Sign in",
  "login.offline": {
    one: "Listen to the {count} track kept in this browser",
    other: "Listen to the {count} tracks kept in this browser",
  },
} as const;
