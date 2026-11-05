# Downloads Wallpaper Engine and its most subscribed Workshop scenes through steamcmd, for
# examples/survey.rs. You log in to steamcmd yourself, in this terminal; the script never sees
# the password or the Steam Guard code. Needs an account that owns Wallpaper Engine.
#
#   python3 tools/survey-set.py --user NAME [--count 60] [--into ~/we]
#
# Use --list to only print the scene ids it would fetch.
import argparse, re, shutil, subprocess, sys, urllib.request
from pathlib import Path

APP = "431960"
BROWSE = ("https://steamcommunity.com/workshop/browse/?appid=" + APP
          + "&requiredtags%5B%5D=Scene&browsesort={sort}&actualsort={sort}&numperpage=30&p={page}")

parser = argparse.ArgumentParser(description="Fetch Wallpaper Engine and popular scenes for the survey")
parser.add_argument("--user", help="Steam account that owns Wallpaper Engine")
parser.add_argument("--count", type=int, default=60, help="how many scenes (default 60)")
parser.add_argument("--sort", default="totaluniquesubscribers", choices=("totaluniquesubscribers", "trend", "mostrecent"))
parser.add_argument("--into", default=str(Path.home() / "we"), help="where steamcmd puts everything (default ~/we)")
parser.add_argument("--list", action="store_true", help="only print the scene ids")
args = parser.parse_args()
if not args.list and not args.user:
    parser.error("--user is needed to download")
if not 1 <= args.count <= 1000:
    parser.error("--count must be between 1 and 1000")

ids = []
for page in range(1, args.count // 30 + 3):
    request = urllib.request.Request(BROWSE.format(sort=args.sort, page=page), headers={"User-Agent": "Mozilla/5.0"})
    html = urllib.request.urlopen(request, timeout=30).read().decode("utf-8", "replace")
    found = [item for item in dict.fromkeys(re.findall(r"filedetails/\?id=(\d+)", html)) if item not in ids]
    if not found:
        break
    ids += found
    if len(ids) >= args.count:
        break
ids = ids[:args.count]
if not ids:
    sys.exit("survey-set: the Workshop listed no scenes")
if args.list:
    print("\n".join(ids))
    sys.exit()

steamcmd = shutil.which("steamcmd") or sys.exit("survey-set: steamcmd isn't installed")
into = Path(args.into).expanduser()
print(f"Fetching Wallpaper Engine and {len(ids)} scenes into {into}; scenes are often 10-200 MB each.")
command = [steamcmd, "+@sSteamCmdForcePlatformType", "windows", "+force_install_dir", str(into),
           "+login", args.user, "+app_update", APP]
for item in ids:
    command += ["+workshop_download_item", APP, item]
code = subprocess.run(command + ["+quit"]).returncode
content = into / "steamapps" / "workshop" / "content" / APP
print(f"\nsteamcmd exited with {code}. Then:\n"
      f"  cargo run --release --example survey -- {content} {into / 'assets'} > survey.md")
