// Scan progress, accounts and tokens, maintenance and missing files.
export const maintenance = {
  "scan.connecting": "Waiting for the first reading…",
  "scan.queued": "Queued",
  "scan.running": "Scanning",
  "scan.completed": "Finished",
  "scan.failed": "Failed",
  "scan.progress": "Scan progress",
  "scan.counted": "{processed} of {total} files",
  "scan.tally":
    "{added} added · {updated} updated · {moved} moved · {skipped} unchanged · {errors} errors",
  "admin.scanning": "Scan in progress",
  "admin.nowPlaying": "Playing now",
  "admin.nobodyListening": "Nobody is listening right now.",
  "admin.tokens": "API tokens for {username}",
  "admin.tokenName": "What is it for?",
  "admin.tokenCreate": "Issue",
  "admin.tokenOnce": "Copy it now. It is shown once and never again:",
  "admin.tokenError": "The token could not be issued.",
  "admin.tokenRevoke": "Revoke",
  "admin.tokenUsed": "already used",
  "admin.tokenUnused": "never used",
  "admin.tokenNone": "No token on this account.",
  "scan.lost":
    "The progress stream could not be opened. The scan itself is unaffected.",
  "admin.members": "Who may see this library",
  "admin.memberRole": "Role",
  "admin.memberRemove": "Remove",
  "admin.memberOwner": "owner, and stays one",
  "admin.memberAdd": "Give access to",
  "admin.memberChoose": "Choose an account…",
  "admin.memberError": "That membership could not be changed.",
  "admin.manage": "Name, folder and deletion",
  "admin.rename": "Name",
  "admin.path": "Folder on the server",
  "admin.followLinks":
    "Follow symbolic links whose target stays inside the library",
  "admin.watch":
    "Watch the folder and scan what changes (off for a network share changed by other machines)",
  "admin.save": "Save",
  "admin.saved": "Saved.",
  "admin.savedScan": "Saved. Scan {id} queued.",
  "admin.saveError":
    "That change could not be saved. A folder must exist on the server and must not be a symbolic link.",
  "admin.saveBusy":
    "The library is being scanned, or another library already uses that folder. Try again once the scan is over.",
  "admin.delete": "Delete this library",
  "admin.deleteNote":
    "Removes the library from the catalogue for every member: its tracks, albums and artists, with their playlist entries, favourites and plays. No file is touched on disk. Type the library’s name to confirm.",
  "admin.deleteConfirm": "The library’s name, to confirm",
  "admin.deleteButton": "Delete for good",
  "admin.deleted": "{name} was deleted: {tracks} tracks left the catalogue.",
  "admin.deleteError": "The library could not be deleted.",
  "scan.leftOut": "{excluded} left out · {links} links not followed",
  "nav.missing": "Missing files",
  "nav.maintenance": "Maintenance",
  "maintenance.sections": "Maintenance sections",
  "maintenance.none": "Nothing here is yours to maintain in this library.",
  "missing.total": {
    one: "{count} file the scans can no longer find",
    other: "{count} files the scans can no longer find",
  },
  "missing.nothing": "Every file is where the catalogue expects it.",
  "missing.notYours":
    "Only the owners and managers of this library, and administrators, see its missing files.",
  "missing.note":
    "These tracks stay in the catalogue, with their favourites, playlist entries and plays, until they are purged; a file that comes back first keeps its track. A purge can be taken back for a while, then the track, and everything that hangs off it for every member, goes for good.",
  "missing.since": "missing since {date}",
  "missing.cost":
    "{playlists} in playlists · {favorites} favourites · {plays} plays",
  "missing.select": "Select {title}",
  "missing.purgeSelected": "Purge the selected ({count})",
  "missing.purgeAll": "Purge them all",
  "missing.confirm": "Confirm: purge {count}",
  "missing.purged": {
    one: "{count} track purged.",
    other: "{count} tracks purged.",
  },
  "missing.scanning":
    "The library is being scanned. Try again once the scan is over.",
  "missing.failed": "The purge did not work. Try again.",
  "missing.more": "Show more",
  "missing.purgedUntil": {
    one: "{count} track purged. It can be restored until {date}.",
    other: "{count} tracks purged. They can be restored until {date}.",
  },
  "missing.purgedTitle": {
    one: "Purged, still restorable ({count})",
    other: "Purged, still restorable ({count})",
  },
  "missing.purgedNote":
    "Nothing of these is lost yet. Restored, a track goes back to the missing files with everything that hung off it; a file that comes back restores its track by itself.",
  "missing.deletedAfter": "deleted for good after {date}",
  "missing.restore": "Restore",
  "missing.restoreOne": "Restore {title}",
  "missing.restoreAll": "Restore them all",
  "missing.restored": {
    one: "{count} track restored.",
    other: "{count} tracks restored.",
  },
  "missing.restoreFailed": "The restoration did not work. Try again.",
} as const;
