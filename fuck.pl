#!/bin/perl

use strict;

# this script renames files in dirs with names like FOOBAR_3021.tiff to LRT_3021.tiff
# FOOBAR can be anything that does not include a '_'

my $dirname = shift;

opendir my $dir, $dirname or warn "cannot open dir: $!\n";

print "opened dir $dirname\n"; 
foreach my $filename (readdir $dir) {
  if($filename =~ /^([^_]+)_(\d+)[.](.*)$/) {
    my ($prefix, $number, $suffix) = ($1, $2, $3);
    system "mv $dirname/$filename $dirname/LRT_$number.$suffix"
  }
}

close $dir;
