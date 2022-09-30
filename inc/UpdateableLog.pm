package UpdateableLog;

use UpdateableLogLine;

use strict;

# a log system that allows updates to log lines after logging

sub new {
  my ($class) = @_;

  my $self =
    {
     'hash', {},		# a hash of log lines by name
     'list', [],		# a list of log lines by position on screen
     };

  return bless $self, $class;
}

sub log($$$) {
  my ($self, $name, $message) = @_;

  my $logline = UpdateableLogLine->new($name, $message);

  my $found = 0;

  my $index = scalar(@{$self->{list}});
  # first look at is it in the list
  foreach my $line (@{$self->{list}}) {
    if($line->{name} eq $name) {
      $found = 1;
      $line->{message} = $message;
      for(my $i = 0 ; $i < $index ; $i++) {
	print "\033[F"; # move cursor to beginning of previous line
      }
      print "$message\n";
      for(my $i = 0 ; $i < $index ; $i++) {
	print "\033[E"; # move cursor to beginning of next line
      }
      # update here
    }
    $index--;
  }

  if(!$found) {
    push @{$self->{list}}, $logline;

    print "$message\n";
  }
}

1;
