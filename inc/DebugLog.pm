package DebugLog;

use strict;

# a basic debug log

sub new {
  my ($class) = @_;

  my $self =
    {
    };

  return bless $self, $class;
}

sub timeLog($$$) {
  my ($self, $name, $message) = @_;

  my $d = `date "+%r"`;
  chop $d;

  $self->log($name, "$d - $message");
}

sub log($$$) {
  my ($self, $name, $message) = @_;

  print "$message\n";
}

1;
