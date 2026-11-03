# Iteration 4: portfolio, then automation

Owner (2026-11-02): portfolio first. `.mpp` reading stays last.

## Portfolio (Window > Portfolio, ⌘0)

1. **[Done]** **Projects:** every project at a glance: status (On track / At risk / Late / Done,
   with reasons), finish, slip against the baseline, % complete (duration-weighted), cost
   against baseline, next milestone, team or local. Rules in `pm_core::portfolio`.
   Projects shown: recent ones, team projects on this Mac, and any added; Remove takes one
   out (the file is not touched).
2. **[Done]** **Timeline:** every project as a bar on one time scale, with progress, milestones,
   the baseline finish and today.
3. **[Done]** **People:** each person's hours per week across all projects (matched by
   resource name), red where it is more than they have.
4. **[Done]** **Filters, groups, totals:** filter by status, team or local, and group (the
   filter applies to Timeline and People too, and is remembered on this Mac). Type a group
   ("Clients", "Internal") in a project's Group cell; Show by group lists each group with its
   own total. Totals add cost per currency; "budget" is the baseline cost, compared with the
   current cost of the projects that have one (projects without a baseline are counted apart).
   Groups are kept on this Mac (portfolio.json), not in the project files.
5. **[Done]** **The team's projects:** when signed in to a team, the portfolio also lists every
   project of your teams (with the team's name), including ones never opened on this Mac:
   those are read from the server, marked "not opened on this Mac yet", and downloaded when
   opened. Offline, a note says they are missing; the rest still shows.
6. **Later:** groups shared by the team (today each person's groups are their own).

## Automation (after the portfolio)

Rules such as "when a task is 100% done, post in its channel", "every Monday, update progress
and send a status summary", recurring tasks, reminders before deadlines.
