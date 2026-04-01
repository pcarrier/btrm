function fish_greeting
    set -l cols (tput cols 2>/dev/null; or echo 80)
    while read -l line
        set -l len (string length -- $line)
        set -l pad (math "$cols - $len")
        if test $pad -gt 0
            printf '%*s%s\n' $pad '' $line
        else
            echo $line
        end
    end </etc/blit-banner.txt
end
