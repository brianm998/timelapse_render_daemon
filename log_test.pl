#!/usr/bin/perl

use strict;
use File::Basename;
use Cwd;
use Getopt::Long qw(GetOptionsFromArray);

my $script_dir;	# figure out where this running script exists on disk
BEGIN { $script_dir = Cwd::realpath( File::Basename::dirname(__FILE__)) }

# add this dir to the include path
use lib "${script_dir}/inc";

use UpdateableLog;

my $log = UpdateableLog->new();

$log->log("crapp", "first");
$log->log("crapp", "second");
$log->log("crapp", "third");
$log->log("foobar", "hello");
$log->log("foobaz", "-----");

sleep 1;

$log->log("foobar", "hello there");

sleep 1;

$log->log("foobar", "hello there.");

sleep 1;

$log->log("foobar", "hello there..");

sleep 1;

$log->log("foobar", "hello there...");
