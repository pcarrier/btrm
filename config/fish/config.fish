function fish_greeting
    set -l line1 "What would be fun to test"
    set -l line2 "a remote terminal instead?"
    set -l cols (tput cols 2>/dev/null; or echo 80)
    set -l pad1 (math $cols - (string length -- $line1))
    set -l pad2 (math $cols - (string length -- $line2))
    if test $pad1 -gt 0
        printf '%*s\n' $cols $line1
    else
        echo $line1
    end
    if test $pad2 -gt 0
        printf '%*s\n' $cols $line2
    else
        echo $line2
    end
end
