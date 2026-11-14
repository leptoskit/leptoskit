---
"leptoskit_admin": minor
---

# An admin for your site

`leptoskit_admin` is one web app for looking after a deployed leptoskit site: its infrastructure, its analytics, and short lessons in both. It is its own app with its own address, and people sign in through your site's own sign-in: they press **Continue**, sign in on your site, and come back. Only accounts your site gives the `admin` role (or the roles you pass to `.roles(…)`) get in; anyone else sees who they signed in as, can switch account, and can read how admins are chosen.

Infrastructure shows what's serving, the changes waiting in `leptoskit.toml` with the plan to review, releases, operations, health from your servers' route metrics, backups, and the configuration. Plan, apply, deploy, first release, maintenance, roll back, health checks, and backup verification each ask you to confirm, run on the deployment service's worker under your name, and stream their progress to the page. Analytics teaches until your app sends its first typed event, and Learn's lessons tick themselves when they become true of your projects. Every card has a **Workings** drawer that walks it back from the check, to the command, to the plan, to the generated `main.tf.json`, to the `leptoskit.toml` lines that declare it.

Build one with `Admin::builder(app_name)`, `.main_site(MainSite::new(issuer, client_id, secret))`, `.keys(…)`, `.data_dir(…)`, and `.control(service)`, then serve `admin.router()`. The `admin_demo` example runs a site and its admin side by side.
