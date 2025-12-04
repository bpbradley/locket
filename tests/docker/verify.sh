#!/bin/sh
# Quick and dirty verification script to cat out all files
# I will improve later to have secret file mappings constructed from
# env vars which can be checked in this script.

SECRETS_ROOT="/out"
EXIT_CODE=0

echo "=== locket ==="

# Iterate over each provider directory mounted in /out/
for provider_path in "$SECRETS_ROOT"/*; do
    if [ -d "$provider_path" ]; then
        PROVIDER_NAME=$(basename "$provider_path")
        echo "== PROVIDER: $PROVIDER_NAME =============================="

        find "$provider_path" -type f | while read -r file; do
            # Make the path relative for cleaner logs (optional)
            REL_PATH="${file#$provider_path/}"
            
            echo "--- $REL_PATH ---"
            if [ -f "$file" ]; then
                cat "$file" && echo ""
            else
                echo "Error reading $file"
                EXIT_CODE=1
            fi
        done
        
        # Check if the directory was empty
        if [ -z "$(ls -A "$provider_path")" ]; then
             echo "WARNING: Directory is empty."
        fi
    fi
done

echo ""
echo "==================================================="
exit $EXIT_CODE