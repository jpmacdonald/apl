import json
import urllib.request
import os

TOP_N = 50
REGISTRY_DIR = "../apl-packages/packages"

def get_top_packages():
    url = "https://formulae.brew.sh/api/analytics/install/30d.json"
    print(f"Fetching {url}...")
    with urllib.request.urlopen(url) as response:
        data = json.loads(response.read())
        # Data is list of { "formula": "name", "count": "123" }
        # specific structure: { "items": [ ... ] } or just list?
        # looking at previous output, it seems to be { "items": [...] } wrapper? 
        # Actually the tail shows [{"number":...}] inside the root list.
        # Let's assume it is a list of objects.
        if isinstance(data, dict) and "items" in data:
            data = data["items"]
        
        # Sort by count desc just in case (though usually pre-sorted)
        # Handle string counts with commas
        data.sort(key=lambda x: int(x["count"].replace(",", "")), reverse=True)
        return [item["formula"] for item in data[:TOP_N]]

def get_existing_packages():
    existing = set()
    if not os.path.exists(REGISTRY_DIR):
        print(f"Warning: {REGISTRY_DIR} not found")
        return existing
        
    for root, dirs, files in os.walk(REGISTRY_DIR):
        for file in files:
            if file.endswith(".toml"):
                # package name is filename base
                existing.add(file.replace(".toml", ""))
    return existing

def main():
    top = get_top_packages()
    existing = get_existing_packages()
    
    missing = []
    print(f"\nScanning Top {TOP_N} Homebrew Packages...")
    for pkg in top:
        # Normalize name (brew often uses full names e.g. "python@3.9", we might just want "python")
        simple_name = pkg.split("@")[0].split("/")[-1]
        
        if simple_name in existing or pkg in existing:
            print(f"  [x] {pkg} (exists)")
        else:
            print(f"  [ ] {pkg} (MISSING)")
            missing.append(pkg)

    print("\nRecommended Import Command:")
    print(f"cargo run --release -p apl-pkg -- import --from homebrew --packages {' '.join(missing[:10])}")

if __name__ == "__main__":
    main()
