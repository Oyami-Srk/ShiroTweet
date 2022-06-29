#!/usr/bin/env zsh
if [[ ! -f $1 ]]; then
    echo "Must specified a TwitterDB SQLite file."
    exit -1
fi
grep -oP "https://twitter.com/.*?\d+?\b" | grep -oP "\d+$" |
    while read line
    do
        sqlite3 $1 "SELECT author, id, content FROM tweet WHERE id=$line;" ".exit" | 
            sed "0,/|/{s/|/\//};s/|/: /g"    
        sqlite3 $1 "DELETE FROM tweet WHERE id=$line;" ".exit"
    done
