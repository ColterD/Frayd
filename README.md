# Frayd

**Your movies, music, books, and comics — all in one place, on your own computer, watchable from any screen in the house.**

Think of it like your own private Netflix, but the "content" is the stuff you already own. No subscriptions, no accounts to sign up for, nothing sent to the internet. It lives on your computer and your family's phones, tablets, and TVs connect to it.

## What can it do?

- **Movies & Shows** — click and watch. It remembers where you paused.
- **Music & Audiobooks** — plays in the background, picks up where you left off.
- **Ebooks** — a real book-reading experience in your browser: pages turn, text looks nice, and you can highlight passages.
- **Comics & Manga** — swipe through pages, remembers your page.
- **Watch Parties** — everyone in the house watches the same thing at the same time, and anyone can add songs/videos to the queue from their phone.

## How do I get it?

You need two things: **this program** and **a folder with your media in it** (your video/music/book files).

1. Go to the installer page and pick your computer type (Windows, Mac, or Linux).
2. Type in where your media folder is — for example `C:\Users\you\Media`.
3. Copy the one command it gives you and paste it into a terminal window. Don't worry — this is the only technical step, and it does everything for you: installs what's needed, sets itself up, and starts automatically whenever you turn your computer on. Takes a few minutes.
4. When the page says your server is ready, click the link, pick a username and password, and you're in. **You're the owner** — only you can change settings.

That's it. After that, everyone in your house just opens the browser, types in your address, and signs in with an account you give them.

## Is it safe?

- Everything stays **on your hardware**. Nothing is uploaded anywhere, and there are no hidden accounts.
- People can only connect from inside your home network unless *you* deliberately open it up.
- Passwords are stored securely (scrambled beyond recognition), and anyone guessing at your login gets locked out after 10 tries.

## Can I share it with friends outside my house?

Yes — that's an advanced step and it's your call. The settings panel has the options (port forwarding, your own web address, security certificates), and there are extra safeguards for when you do.

## Under the hood (for the curious)

Rust server (axum + SQLite) and a plain JavaScript web client. No telemetry, no dependencies phoning home, small enough for a Raspberry Pi. Free and open source.
